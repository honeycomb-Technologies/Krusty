use super::*;

impl App {
    /// Create new app instance
    pub async fn new() -> Self {
        let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        // Initialize all services via builder
        let (
            services,
            channels,
            process_registry,
            current_model,
            theme,
            theme_name,
            active_provider,
        ) = crate::tui::app_builder::init_services(&working_dir).await;

        let ui = AppUi::new(theme, theme_name, working_dir.clone());
        let runtime = AppRuntime::new(
            current_model,
            active_provider,
            working_dir,
            process_registry,
        );

        // Manually set channels that were initialized in init_services
        let runtime = AppRuntime {
            channels,
            ..runtime
        };

        let mut app = Self {
            ui,
            runtime,
            services,
        };

        // Prime installable plugin catalog before first render.
        app.refresh_plugin_catalog(false);
        app
    }

    /// Run the application
    pub async fn run(&mut self) -> Result<()> {
        let _ = self.try_load_auth().await;

        // Check if we just applied an update (marker file written by apply_pending_update)
        if let Some(version) = krusty_core::updater::read_update_marker() {
            self.show_toast(crate::tui::components::Toast::info(format!(
                "Updated to v{}",
                version
            )));
            self.runtime.just_updated = true;
        }

        // Check for pending update from previous session (cleans up stale files)
        self.check_pending_update();

        // Check for updates in background
        self.start_update_check();

        // Start background refresh of OpenRouter models if configured and cache is stale
        if self.should_refresh_dynamic_models(self.runtime.active_provider) {
            tracing::info!(
                "Starting background {:?} model refresh",
                self.runtime.active_provider
            );
            self.start_dynamic_model_fetch(self.runtime.active_provider);
        }

        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste,
            // Enable Kitty keyboard protocol for better key detection
            // - DISAMBIGUATE_ESCAPE_CODES: Better escape sequence handling
            // - REPORT_EVENT_TYPES: Enables key release detection (needed for games)
            // Note: REPORT_ALL_KEYS_AS_ESCAPE_CODES breaks Shift+key for special chars
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Initialize Kitty graphics support for plugin window
        self.ui.plugin_window.detect_graphics_support();
        self.ui.plugin_window.update_cell_size();

        let result = self.main_loop(&mut terminal).await;

        // Kill all background processes on shutdown
        self.runtime.process_registry.kill_all().await;

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            PopKeyboardEnhancementFlags,
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste
        )?;
        terminal.show_cursor()?;
        result
    }

    /// Process a single terminal event
    fn process_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                self.handle_key(key);
                self.ui.needs_redraw = true;
            }
            Event::Mouse(mouse) => {
                self.handle_mouse_event(mouse);
                self.ui.needs_redraw = true;
            }
            Event::Paste(text) => {
                self.handle_paste(text);
                self.ui.needs_redraw = true;
            }
            Event::Resize(_, _) => {
                // Update cell size for Kitty graphics on resize
                self.ui.plugin_window.update_cell_size();
                self.ui.needs_redraw = true;
            }
            _ => {}
        }
    }

    /// Main event loop
    async fn main_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        // Use async event stream to avoid blocking the runtime
        // This fixes the issue where the app freezes when mouse leaves terminal
        let mut event_stream = EventStream::new();

        loop {
            if let Some(area) = self.ui.scroll_system.layout.input_area {
                self.ui.input.set_width(area.width);
            }

            // Update running process count and elapsed time for status bar (non-blocking)
            if let Some(count) = self.runtime.process_registry.try_running_count() {
                self.runtime.running_process_count = count;
            }
            self.runtime.running_process_elapsed =
                self.runtime.process_registry.try_oldest_running_elapsed();

            // Refresh git status for status bar (throttled inside handler).
            self.poll_git_status();
            self.poll_plugin_catalog();

            // Keep process popup updated while open (non-blocking)
            if self.ui.popup == Popup::ProcessList {
                if let Some(processes) = self.runtime.process_registry.try_list() {
                    self.ui.popups.process.update(processes);
                }
            }

            // Process core orchestrator events (LoopEvent channel)
            if self.process_loop_events() {
                self.ui.needs_redraw = true;
            }

            // Check for timed-out tool approvals
            self.check_approval_timeout();

            // Poll async operations
            self.poll_dynamic_model_fetch();
            self.poll_title_generation();
            self.poll_compaction();

            // Update menu animations (only when on start menu for efficiency)
            if self.ui.view == View::StartMenu {
                // Use inner_area width (terminal width minus borders) so crab stays contained
                let term_size = terminal.size()?;
                let inner_width = term_size.width.saturating_sub(2); // Account for logo border
                self.ui.menu_animator.update(
                    inner_width,
                    term_size.height,
                    Duration::from_millis(16),
                );
            }

            // Poll bash output channel for streaming updates
            self.poll_bash_output();

            // Poll delegated/or legacy explore/build progress channels for agent updates
            let delegated_result = self.poll_delegated_progress();
            if delegated_result.needs_redraw {
                self.ui.needs_redraw = true;
            }
            self.process_poll_actions(delegated_result);

            let explore_result = self.poll_explore_progress();
            if explore_result.needs_redraw {
                self.ui.needs_redraw = true;
            }
            self.process_poll_actions(explore_result);

            let build_result = self.poll_build_progress();
            if build_result.needs_redraw {
                self.ui.needs_redraw = true;
            }
            self.process_poll_actions(build_result);

            // Poll /init exploration progress and result
            // Clone cached languages to avoid borrow conflict (cleared on completion)
            let languages = self
                .runtime
                .cached_init_languages
                .clone()
                .unwrap_or_default();
            let init_result = poll_init_exploration(
                &mut self.runtime.channels,
                &mut self.runtime.blocks.explore,
                &mut self.runtime.init_explore_id,
                &mut self.runtime.cached_init_languages,
                &self.runtime.working_dir,
                &languages,
            );
            if init_result.needs_redraw {
                self.ui.needs_redraw = true;
            }
            self.process_poll_actions(init_result);

            // Poll MCP status updates from background tasks
            let mcp_result = poll_mcp_status(&mut self.runtime.channels, &mut self.ui.popups.mcp);
            if mcp_result.needs_redraw {
                self.ui.needs_redraw = true;
            }
            self.process_poll_actions(mcp_result);

            // Poll OAuth status updates from background tasks
            let oauth_result = poll_oauth_status(
                &mut self.runtime.channels,
                &mut self.ui.popups.auth,
                self.runtime.active_provider,
            );
            if oauth_result.needs_redraw {
                self.ui.needs_redraw = true;
            }
            self.process_poll_actions(oauth_result);

            // Poll update status and show toasts
            self.poll_update_status();

            // Poll terminal panes for output updates and cursor blink
            self.poll_terminal_panes();

            // Poll ProcessRegistry for background process status updates
            let process_result = poll_background_processes(
                &self.runtime.process_registry,
                &mut self.runtime.blocks.bash,
            );
            if process_result.needs_redraw {
                self.ui.needs_redraw = true;
            }

            // Tick toasts (auto-dismiss expired) - mark dirty if any expired
            if self.ui.toasts.tick() {
                self.ui.needs_redraw = true;
            }

            // Tick all animation blocks (before render, not during)
            // Returns true if any block is still animating
            if self.tick_blocks() {
                self.ui.needs_redraw = true;
            }

            // Process continuous edge scrolling during selection
            if self.ui.scroll_system.edge_scroll.direction.is_some() {
                self.process_edge_scroll();
                self.ui.needs_redraw = true;
            }

            // Always redraw if streaming is active (receiving deltas)
            if self.runtime.chat.is_streaming || self.runtime.chat.has_stream_backlog() {
                self.ui.needs_redraw = true;
            }

            // Only render if something changed
            if self.ui.needs_redraw {
                terminal.draw(|f| self.ui(f))?;
                // Flush any pending Kitty graphics after buffer is rendered
                self.ui.plugin_window.flush_pending_graphics();
                self.ui.needs_redraw = false;
            }

            // 60fps polling - edge scroll needs faster polling for smooth scrolling
            let poll_timeout = if self.ui.scroll_system.edge_scroll.direction.is_some()
                || self.runtime.chat.has_stream_backlog()
            {
                Duration::from_millis(8) // faster polling for edge scrolling and stream backlog drain
            } else {
                Duration::from_millis(16) // 60fps normal
            };

            // Async event handling - doesn't block the runtime when no events
            // This allows async tasks to progress even when mouse is outside terminal
            tokio::select! {
                biased; // Prefer events over timeout when both are ready

                maybe_event = event_stream.next() => {
                    if let Some(Ok(event)) = maybe_event {
                        self.process_event(event);

                        // Drain all pending events for snappy scrollbar dragging
                        // This prevents event queue buildup during rapid mouse movements
                        while let Ok(Some(Ok(event))) = tokio::time::timeout(
                            Duration::ZERO,
                            event_stream.next()
                        ).await {
                            self.process_event(event);
                        }
                    }
                }
                _ = tokio::time::sleep(poll_timeout) => {
                    // Timeout - continue loop for regular updates (animations, polling, etc.)
                }
            }

            // Apply any deferred view changes (after popup handling)
            self.apply_pending_view_change();

            if self.runtime.should_quit {
                // Save session state before exiting
                self.save_session_token_count();
                self.save_block_ui_states();
                break;
            }
        }
        Ok(())
    }
}
