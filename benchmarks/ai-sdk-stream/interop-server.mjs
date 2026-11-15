// A real AI SDK 7 backend for interop tests: `streamText` over a mock language model (no provider, no network),
// answering `POST /api/chat` with the SDK's own UI message stream. The request body is handed to `streamText`
// as `messages`, exactly as an assistant-ui data-stream backend does, so a malformed request fails here.
//
//   node interop-server.mjs        prints `LISTENING <port>` once ready
//   GET /last-request              the last request body as received
//   GET /last-prompt               the prompt the model saw after the SDK converted the request
import http from 'node:http';
import { streamText, simulateReadableStream, jsonSchema, tool, stepCountIs } from 'ai';
import { MockLanguageModelV4 } from 'ai/test';

const usage = { inputTokens: { total: 3, noCache: 3 }, outputTokens: { total: 5, text: 5 } };
let lastRequest = null;
let lastPrompt = null;

function model() {
  let call = 0;
  return new MockLanguageModelV4({
    doStream: async (options) => {
      call += 1;
      if (call === 1) lastPrompt = options.prompt;
      const parts =
        call === 1
          ? [
              { type: 'stream-start', warnings: [] },
              { type: 'reasoning-start', id: 'r' },
              { type: 'reasoning-delta', id: 'r', delta: 'Thinking it over.' },
              { type: 'reasoning-end', id: 'r' },
              { type: 'tool-input-start', id: 'c1', toolName: 'weather' },
              { type: 'tool-input-delta', id: 'c1', delta: '{"city":' },
              { type: 'tool-input-delta', id: 'c1', delta: '"Paris"}' },
              { type: 'tool-input-end', id: 'c1' },
              { type: 'tool-call', toolCallId: 'c1', toolName: 'weather', input: '{"city":"Paris"}' },
              { type: 'finish', finishReason: { unified: 'tool-calls', raw: 'tool_calls' }, usage },
            ]
          : [
              { type: 'stream-start', warnings: [] },
              { type: 'text-start', id: 't' },
              { type: 'text-delta', id: 't', delta: 'It is ' },
              { type: 'text-delta', id: 't', delta: '21 degrees.' },
              { type: 'text-end', id: 't' },
              { type: 'finish', finishReason: { unified: 'stop', raw: 'stop' }, usage },
            ];
      return { stream: simulateReadableStream({ chunks: parts, chunkDelayInMs: 1 }) };
    },
  });
}

const server = http.createServer(async (req, res) => {
  if (req.method === 'GET' && req.url === '/last-request') {
    res.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify(lastRequest));
    return;
  }
  if (req.method === 'GET' && req.url === '/last-prompt') {
    res.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify(lastPrompt));
    return;
  }
  if (req.method !== 'POST' || req.url !== '/api/chat') {
    res.writeHead(404).end();
    return;
  }
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  try {
    lastRequest = JSON.parse(Buffer.concat(chunks).toString('utf8'));
    const result = streamText({
      model: model(),
      messages: lastRequest.messages,
      system: lastRequest.system,
      tools: {
        weather: tool({
          description: 'Weather for a city',
          inputSchema: jsonSchema({
            type: 'object',
            properties: { city: { type: 'string' } },
            required: ['city'],
          }),
          execute: async ({ city }) => ({ city, temp_c: 21 }),
        }),
      },
      stopWhen: stepCountIs(3),
    });
    const response = result.toUIMessageStreamResponse();
    res.writeHead(response.status, Object.fromEntries(response.headers));
    const reader = response.body.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      res.write(value);
    }
    res.end();
  } catch (error) {
    res.writeHead(500, { 'content-type': 'text/plain' }).end(String(error?.message ?? error));
  }
});

server.listen(0, '127.0.0.1', () => {
  console.log(`LISTENING ${server.address().port}`);
});
