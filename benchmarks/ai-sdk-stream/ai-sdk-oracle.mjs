#!/usr/bin/env node
/**
 * Published AI SDK 7.0.166 oracle. This runs upstream decoding, schema validation,
 * folding and every snapshot emitted by readUIMessageStream. It does not render UI.
 * Sources and exact fixture interpretation are in README.md and ui7.json.
 */
import { readFile } from 'node:fs/promises';
import { once } from 'node:events';
import { createRequire } from 'node:module';
import { performance } from 'node:perf_hooks';
import { resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
import { isDeepStrictEqual } from 'node:util';
import { readUIMessageStream, uiMessageChunkSchema } from 'ai';
import { parseJsonEventStream } from '@ai-sdk/provider-utils';

const require = createRequire(import.meta.url);
const pinnedVersions = { ai: '7.0.166', '@ai-sdk/provider-utils': '5.0.76' };
for (const [name, expected] of Object.entries(pinnedVersions)) {
  if (require(`${name}/package.json`).version !== expected) {
    throw new Error(`Oracle requires the pinned ${name} version; run npm ci.`);
  }
}

class OracleInputError extends Error {
  constructor(code) {
    super(code);
    this.name = 'OracleInputError';
    this.code = code;
  }
}

function requireString(value, code) {
  if (typeof value !== 'string') throw new OracleInputError(code);
  return value;
}

function decodeBase64(value) {
  const encoded = requireString(value, 'BASE64_CHUNK_NOT_STRING');
  // Buffer.from alone silently accepts malformed base64. The wire fixture must
  // retain exact bytes, so require the canonical padded encoding used by fixtures.
  if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(encoded)) {
    throw new OracleInputError('INVALID_BASE64_CHUNK');
  }
  const bytes = Buffer.from(encoded, 'base64');
  if (bytes.toString('base64') !== encoded) throw new OracleInputError('INVALID_BASE64_CHUNK');
  return bytes;
}

function prepareChunks(request) {
  if (request === null || typeof request !== 'object' || Array.isArray(request)) {
    throw new OracleInputError('REQUEST_NOT_OBJECT');
  }
  if (request.mode !== undefined && typeof request.mode !== 'string') {
    throw new OracleInputError('MODE_NOT_STRING');
  }
  const hasBase64 = request.network_chunks_base64 !== undefined;
  const hasUtf8 = request.network_chunks_utf8 !== undefined;
  if (hasBase64 && hasUtf8) throw new OracleInputError('AMBIGUOUS_NETWORK_CHUNKS');
  if (request.chunk_bytes !== undefined &&
      (!Number.isSafeInteger(request.chunk_bytes) || request.chunk_bytes <= 0)) {
    throw new OracleInputError('INVALID_CHUNK_BYTES');
  }
  const encoder = new TextEncoder();
  if (hasBase64 || hasUtf8) {
    const chunks = hasBase64 ? request.network_chunks_base64 : request.network_chunks_utf8;
    if (!Array.isArray(chunks)) throw new OracleInputError('NETWORK_CHUNKS_NOT_ARRAY');
    return hasBase64
      ? chunks.map(decodeBase64)
      : chunks.map(chunk => encoder.encode(requireString(chunk, 'UTF8_CHUNK_NOT_STRING')));
  }
  const bytes = encoder.encode(requireString(request.raw_utf8, 'RAW_UTF8_NOT_STRING'));
  if (request.chunk_bytes === undefined || bytes.length === 0) return [bytes];
  const chunks = [];
  for (let offset = 0; offset < bytes.length; offset += request.chunk_bytes) {
    chunks.push(bytes.subarray(offset, offset + request.chunk_bytes));
  }
  return chunks;
}

/** Keep precisely the stable error projection used in the established corpus. */
export function normalizedError(error) {
  const result = { name: error?.name ?? 'Error' };
  if (result.name === 'AI_UIMessageStreamError') {
    result.chunkType = error.chunkType;
    if (error.chunkId !== undefined) result.chunkId = error.chunkId;
  }
  if (result.name === 'Error') result.message = error?.message;
  return result;
}

/**
 * Decode and fold one in-memory network replay. `mode` is accepted for a shared
 * benchmark request format, but this oracle ALWAYS executes upstream snapshots.
 * elapsed_ms excludes request JSON parsing, byte preparation, initial-message
 * copying, and result JSON serialization. It includes stream setup, SSE/JSON/schema
 * decoding, sidecar observation, reducer work and upstream snapshot production.
 */
export async function replay(request) {
  const chunks = prepareChunks(request);
  const initial = request.initialMessage ?? { id: 'm', role: 'assistant', parts: [] };
  if (initial === null || typeof initial !== 'object' || Array.isArray(initial) ||
      typeof initial.id !== 'string' || !Array.isArray(initial.parts) ||
      !['assistant', 'user', 'system'].includes(initial.role)) {
    throw new OracleInputError('INVALID_INITIAL_MESSAGE');
  }
  const message = structuredClone(initial);
  let last = structuredClone(initial);
  const errors = [];
  const transientData = [];
  const aborts = [];
  let finishReason;
  let emissions = 0;

  const start = performance.now();
  const input = new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(chunk);
      controller.close();
    },
  });
  const parsed = parseJsonEventStream({ stream: input, schema: uiMessageChunkSchema })
    .pipeThrough(new TransformStream({
      transform(result, controller) {
        if (!result.success) throw result.error;
        const chunk = result.value;
        if (chunk.type.startsWith('data-') && chunk.transient) transientData.push(chunk);
        if (chunk.type === 'finish' && chunk.finishReason != null) finishReason = chunk.finishReason;
        if (chunk.type === 'abort') aborts.push(chunk);
        controller.enqueue(chunk);
      },
    }));
  for await (const snapshot of readUIMessageStream({
    message,
    stream: parsed,
    onError: error => errors.push(normalizedError(error)),
    // Deliberately omit terminateOnError: the published helper default is false.
  })) {
    last = snapshot;
    emissions += 1;
  }
  const elapsed_ms = performance.now() - start;
  // JSON normalization is outside the measured interval. Upstream leaves
  // undefined-valued properties on objects; the stored fixture projection omits them.
  return JSON.parse(JSON.stringify({
    foldedMessage: last,
    sidecars: { errors, transientData, aborts, ...(finishReason !== undefined ? { finishReason } : {}) },
    elapsed_ms,
    emissions,
  }));
}

async function writeJson(value) {
  if (!process.stdout.write(`${JSON.stringify(value)}\n`)) await once(process.stdout, 'drain');
}

async function verifyFixtures(path) {
  const fixtureUrl = path === undefined
    ? new URL('../../crates/ai-ui-sdk-stream/tests/fixtures/ui7.json', import.meta.url)
    : resolve(path);
  const corpus = JSON.parse(await readFile(fixtureUrl, 'utf8'));
  if (corpus.format_version !== 1 || corpus.oracle_profile !== 'ai-sdk-ui7' ||
      !Array.isArray(corpus.vectors) || corpus.vectors.length === 0 ||
      corpus.counts?.ESTABLISHED !== corpus.vectors.length ||
      corpus.vectors.some(fixture => fixture.label !== 'ESTABLISHED' ||
        fixture.oracle_profile !== 'ai-sdk-ui7' || fixture.expected === undefined) ||
      Object.entries(pinnedVersions).some(([name, version]) => corpus.versions?.[name] !== version)) {
    throw new OracleInputError('INVALID_FIXTURE_CORPUS');
  }
  const failures = [];
  let emissions = 0;
  for (const fixture of corpus.vectors) {
    const result = await replay(fixture);
    emissions += result.emissions;
    for (const field of ['foldedMessage', 'sidecars']) {
      if (!isDeepStrictEqual(result[field], fixture.expected[field])) {
        failures.push({ id: fixture.id, field });
      }
    }
  }
  await writeJson({
    oracle: 'ai-sdk-ui7',
    versions: pinnedVersions,
    fixtures: corpus.vectors.length,
    passed: corpus.vectors.length - new Set(failures.map(failure => failure.id)).size,
    emissions,
    failures,
  });
  if (failures.length !== 0) process.exitCode = 1;
}

async function main() {
  if (process.argv[2] === '--verify') {
    await verifyFixtures(process.argv[3]);
    return;
  }
  if (process.argv.length > 2) throw new OracleInputError('UNRECOGNIZED_ARGUMENT');
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  for await (const line of lines) {
    if (line.trim().length === 0) continue;
    let request;
    try {
      request = JSON.parse(line);
    } catch {
      await writeJson({ error: { name: 'OracleInputError', code: 'INVALID_REQUEST_JSON' } });
      process.exitCode = 1;
      continue;
    }
    try {
      await writeJson(await replay(request));
    } catch (error) {
      // Protocol errors normally arrive in sidecars through upstream onError.
      // Harness failures expose no raw input, server diagnostic or stack trace.
      await writeJson({ error: {
        name: error instanceof OracleInputError ? error.name : 'OracleFailure',
        code: error instanceof OracleInputError ? error.code : 'REPLAY_FAILED',
      } });
      process.exitCode = 1;
    }
  }
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(async () => {
    await writeJson({ error: { name: 'OracleFailure', code: 'HARNESS_FAILED' } });
    process.exitCode = 1;
  });
}
