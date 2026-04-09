interface TerminalTheme {
  background: string;
  foreground: string;
  cursor: string;
}

export function getTerminalHtml(wsUrl: string, theme: TerminalTheme): string {
  return `<!DOCTYPE html>
<html>
<head>
  <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no">
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5.3.0/css/xterm.min.css">
  <script src="https://cdn.jsdelivr.net/npm/xterm@5.3.0/lib/xterm.min.js"></script>
  <script src="https://cdn.jsdelivr.net/npm/@xterm/addon-fit@0.10.0/lib/addon-fit.min.js"></script>
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
          background: '${theme.background}',
          foreground: '${theme.foreground}',
          cursor: '${theme.cursor}',
        },
        allowProposedApi: true,
      });

      var fitAddon = new window.FitAddon.FitAddon();
      term.loadAddon(fitAddon);
      term.open(document.getElementById('terminal'));
      fitAddon.fit();

      var ws = new WebSocket('${wsUrl}');

      ws.onopen = function() {
        fitAddon.fit();
      };

      ws.onmessage = function(event) {
        term.write(typeof event.data === 'string' ? event.data : new Uint8Array(event.data));
      };

      ws.onclose = function() {
        term.write('\\r\\n[Connection closed]\\r\\n');
      };

      term.onData(function(data) {
        if (ws.readyState === WebSocket.OPEN) ws.send(data);
      });

      term.onResize(function(size) {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: 'resize', cols: size.cols, rows: size.rows }));
        }
      });

      window.addEventListener('resize', function() { fitAddon.fit(); });

      window.ReactNativeWebView && window.ReactNativeWebView.postMessage(JSON.stringify({ type: 'ready' }));
    })();
  </script>
</body>
</html>`;
}
