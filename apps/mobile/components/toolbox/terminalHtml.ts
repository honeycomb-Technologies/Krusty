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
      var heartbeat = null;

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
          term.write(new Uint8Array(event.data));
          return;
        }

        try {
          var message = JSON.parse(event.data);
          if (message.type === 'output' && typeof message.data === 'string') {
            term.write(message.data);
            return;
          }
          if (message.type === 'error' && typeof message.error === 'string') {
            term.write('\\r\\n' + message.error + '\\r\\n');
            return;
          }
          if (message.type === 'pong') {
            return;
          }
        } catch (error) {
          term.write(event.data);
          return;
        }
      };

      ws.onclose = function() {
        if (heartbeat) {
          clearInterval(heartbeat);
          heartbeat = null;
        }
        term.write('\\r\\n[Connection closed]\\r\\n');
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
