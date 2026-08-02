#!/usr/bin/env node
// Live Chat stream smoke against local Mitsuro server.
// Usage: node scripts/smoke-chat-stream.mjs

const base = process.env.KRUSTY_URL || 'http://127.0.0.1:3000';
const token = process.env.KRUSTY_TOKEN || 'local';

async function main() {
  const headers = {
    Authorization: `Bearer ${token}`,
    'Content-Type': 'application/json',
    Accept: 'text/event-stream',
  };

  const createRes = await fetch(`${base}/api/sessions`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      title: 'Desktop stream smoke',
      workspace_mode: process.env.MITSURO_SMOKE_DIR ? 'selected' : 'neutral',
      ...(process.env.MITSURO_SMOKE_DIR ? { project_dir: process.env.MITSURO_SMOKE_DIR } : {}),
      session_type: 'chat',
      permission_mode: 'autonomous',
    }),
  });
  if (!createRes.ok) {
    throw new Error(`create session failed: ${createRes.status} ${await createRes.text()}`);
  }
  const session = await createRes.json();
  const sessionId = session.id;
  console.log('session', sessionId);

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 45000);
  let text = '';
  let finished = false;
  let error = null;

  try {
    const res = await fetch(`${base}/api/chat`, {
      method: 'POST',
      headers,
      signal: controller.signal,
      body: JSON.stringify({
        session_id: sessionId,
        message: 'Reply with exactly: DESKTOP_STREAM_OK',
        session_type: 'chat',
        thinking_enabled: false,
      }),
    });
    if (!res.ok) {
      throw new Error(`stream failed: ${res.status} ${await res.text()}`);
    }
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const chunks = buffer.split('\n\n');
      buffer = chunks.pop() || '';
      for (const chunk of chunks) {
        const lines = chunk.split('\n');
        let eventType = 'message';
        let data = '';
        for (const line of lines) {
          if (line.startsWith('event:')) eventType = line.slice(6).trim();
          if (line.startsWith('data:')) data += line.slice(5).trim();
        }
        if (!data) continue;
        let payload;
        try {
          payload = JSON.parse(data);
        } catch {
          continue;
        }
        const type = payload.type || eventType;
        if (type === 'text_delta' || type === 'text_delta_with_citations') {
          text += payload.delta || '';
          process.stdout.write(payload.delta || '');
        } else if (type === 'error') {
          error = payload.message || payload.error || JSON.stringify(payload);
        } else if (type === 'done' || type === 'finish' || type === 'completed') {
          finished = true;
        }
      }
      if (finished || error) break;
    }
  } finally {
    clearTimeout(timeout);
    await fetch(`${base}/api/sessions/${sessionId}`, {
      method: 'DELETE',
      headers: { Authorization: `Bearer ${token}` },
    }).catch(() => {});
  }

  console.log('\n---');
  console.log('finished=', finished);
  console.log('error=', error);
  console.log('text=', JSON.stringify(text));
  if (error) process.exit(2);
  if (!text.trim()) process.exit(3);
  if (!text.includes('DESKTOP_STREAM_OK') && text.trim().length < 3) process.exit(4);
  console.log('STREAM_SMOKE_OK');
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
