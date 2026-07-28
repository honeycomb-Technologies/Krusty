interface TerminalTheme {
  background: string;
  foreground: string;
  cursor: string;
}

export function getTerminalHtml(wsUrl: string, theme: TerminalTheme): string {
  const wsUrlJson = JSON.stringify(wsUrl);
  const backgroundJson = JSON.stringify(theme.background);
  const foregroundJson = JSON.stringify(theme.foreground);
  const cursorJson = JSON.stringify(theme.cursor);

  return `<!DOCTYPE html>
<html>
<head>
  <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no">
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5.3.0/css/xterm.min.css">
  <script src="https://cdn.jsdelivr.net/npm/xterm@5.3.0/lib/xterm.min.js"></script>
  <script src="https://cdn.jsdelivr.net/npm/@xterm/addon-fit@0.11.0/lib/addon-fit.min.js"></script>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    html, body { width: 100%; height: 100%; overflow: hidden; background: ${theme.background}; }
    #terminal { width: 100%; height: 100%; }
    .xterm { padding: 4px; }
  </style>
</head>
<body>
  <div id="terminal"></div>
  <script>
    (function() {
      var term = new window.Terminal({
        cursorBlink: true,
        fontSize: 13,
        fontFamily: 'Menlo, Monaco, monospace',
        theme: {
          background: ${backgroundJson},
          foreground: ${foregroundJson},
          cursor: ${cursorJson},
        },
        allowProposedApi: true,
      });

      var fitAddon = new window.FitAddon.FitAddon();
      term.loadAddon(fitAddon);
      term.open(document.getElementById('terminal'));
      fitAddon.fit();

      var ws = new WebSocket(${wsUrlJson});
      ws.binaryType = 'arraybuffer';
      var heartbeat = null;
      var pendingOutput = [];
      var pendingOutputBytes = 0;
      var outputWriteActive = false;
      var outputOverflowed = false;
      var OUTPUT_HIGH_WATERMARK = 512 * 1024;
      var OUTPUT_LOW_WATERMARK = 128 * 1024;

      function outputSize(data) {
        return typeof data === 'string' ? data.length * 2 : (data.byteLength || 0);
      }

      function drainOutput() {
        if (outputWriteActive || pendingOutput.length === 0) return;
        outputWriteActive = true;
        var chunk = pendingOutput.shift();
        pendingOutputBytes = Math.max(0, pendingOutputBytes - outputSize(chunk));
        term.write(chunk, function() {
          outputWriteActive = false;
          if (outputOverflowed && pendingOutputBytes <= OUTPUT_LOW_WATERMARK) {
            outputOverflowed = false;
          }
          drainOutput();
        });
      }

      function enqueueOutput(data) {
        var nextBytes = pendingOutputBytes + outputSize(data);
        if (nextBytes > OUTPUT_HIGH_WATERMARK) {
          if (!outputOverflowed) {
            outputOverflowed = true;
            var warning = '\\r\\n[Terminal output exceeded the safe buffer; reconnecting is required.]\\r\\n';
            pendingOutput.push(warning);
            pendingOutputBytes += outputSize(warning);
            if (ws.readyState === WebSocket.OPEN) {
              ws.close(1008, 'terminal output buffer exceeded');
            }
          }
          drainOutput();
          return;
        }
        pendingOutput.push(data);
        pendingOutputBytes = nextBytes;
        drainOutput();
      }

      function sendJson(payload) {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify(payload));
        }
      }

      function sendResize() {
        sendJson({ type: 'resize', cols: term.cols, rows: term.rows });
      }

      ws.onopen = function() {
        fitAddon.fit();
        sendJson({ type: 'hello', binary_output: false });
        sendResize();
        heartbeat = setInterval(function() {
          sendJson({ type: 'ping' });
        }, 15000);
      };

      ws.onmessage = function(event) {
        if (typeof event.data !== 'string') {
          enqueueOutput(new Uint8Array(event.data));
          return;
        }

        try {
          var message = JSON.parse(event.data);
          if (message.type === 'output' && typeof message.data === 'string') {
            enqueueOutput(message.data);
            return;
          }
          if (message.type === 'error' && typeof message.error === 'string') {
            enqueueOutput('\\r\\n' + message.error + '\\r\\n');
            return;
          }
          if (message.type === 'pong') {
            return;
          }
        } catch (error) {
          enqueueOutput(event.data);
          return;
        }
      };

      ws.onclose = function() {
        if (heartbeat) {
          clearInterval(heartbeat);
          heartbeat = null;
        }
        enqueueOutput('\\r\\n[Connection closed]\\r\\n');
      };

      term.onData(function(data) {
        sendJson({ type: 'input', data: data });
      });

      term.onResize(function(size) {
        sendJson({ type: 'resize', cols: size.cols, rows: size.rows });
      });

      window.addEventListener('resize', function() {
        fitAddon.fit();
        sendResize();
      });

      window.ReactNativeWebView && window.ReactNativeWebView.postMessage(JSON.stringify({ type: 'ready' }));
    })();
  </script>
</body>
</html>`;
}
