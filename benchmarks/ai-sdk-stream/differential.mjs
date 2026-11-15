#!/usr/bin/env node
/**
 * Focused differential verification beyond the 45 checked-in source fixtures.
 *
 * Usage (from the repository root, after installing this directory's locked
 * Node dependencies and building the Rust replay example):
 *   node benchmarks/ai-sdk-stream/differential.mjs /path/to/examples/replay
 *
 * The 257 deterministic synthetic requests exercise tool transitions and late
 * deltas, duplicate/resumed IDs, approvals, metadata, required versus nullable
 * payloads, secure-JSON keys, and incomplete argument previews. The reference
 * is the published ai@7.0.166 / @ai-sdk/provider-utils@5.0.76 oracle imported
 * below. Compare folded messages, sidecars, and publication counts, not timing.
 *
 * Ten narrowly checked Unicode differences are documented profile limitations:
 * JavaScript can expose individual UTF-16 surrogate code units, but Rust JSON
 * strings contain Unicode scalar values. Nine cases spread non-BMP string
 * metadata; one materializes a partial escaped high surrogate. An exception is
 * accepted only when substituting that exact known projection makes the entire
 * output equal. It cannot suppress another field, error, or publication mismatch.
 * Numeric overflow and parser-depth parity are not claimed by this gate.
 *
 * Exit 0 means all comparisons match or are those exact known exceptions.
 * Exit 1 means an unexpected mismatch; exit 2 means a runner/infrastructure error.
 * Diagnostics are bounded and contain synthetic case labels only, never raw
 * replay input, captured stderr, or complete result payloads.
 */

import { spawnSync } from 'node:child_process';
import { isDeepStrictEqual } from 'node:util';
import { replay } from './ai-sdk-oracle.mjs';

const binary = process.argv[2];
if (!binary || process.argv.length !== 3) {
  process.stderr.write('Usage: node differential.mjs /path/to/examples/replay\n');
  process.exitCode = 2;
} else {
  try {
    await verify(binary);
  } catch (error) {
    const kind = error instanceof Error ? error.name : 'UnknownError';
    process.stderr.write(`Differential runner failed (${kind}); no replay payloads were logged.\n`);
    process.exitCode = 2;
  }
}

async function verify(executable) {
  const cases = buildCases();
  const wire = cases.map(({ request }) => JSON.stringify(request)).join('\n') + '\n';
  const run = spawnSync(executable, [], {
    input: wire,
    encoding: 'utf8',
    maxBuffer: 32 * 1024 * 1024,
    timeout: 60_000,
  });
  if (run.error || run.status !== 0) throw new Error('Rust replay process failed');
  const results = run.stdout.trim().split('\n').map(JSON.parse);
  if (results.length !== cases.length) throw new Error('Rust replay result count differs');

  const unexpected = [];
  const known = [];
  for (let index = 0; index < cases.length; index += 1) {
    const test = cases[index];
    const expected = await replay(test.request);
    const actual = results[index];
    for (const field of ['foldedMessage', 'sidecars', 'emissions']) {
      if (isDeepStrictEqual(expected[field], actual[field])) continue;
      const diagnostic = { case: index + 1, name: test.name, field };
      if (isKnownUnicodeDifference(test, field, expected[field], actual[field])) {
        known.push({ ...diagnostic, limitation: test.unicode.kind });
      } else {
        unexpected.push(diagnostic);
      }
    }
  }

  const unexpectedCases = new Set(unexpected.map(item => item.case)).size;
  const knownCases = new Set(known.map(item => item.case)).size;
  const summary = {
    reference: 'ai@7.0.166 / @ai-sdk/provider-utils@5.0.76',
    cases: cases.length,
    exactCases: cases.length - new Set([...known, ...unexpected].map(item => item.case)).size,
    knownUnicodeCases: knownCases,
    unexpectedCases,
    knownDifferences: known.slice(0, 20),
    unexpectedDifferences: unexpected.slice(0, 20),
    omittedDiagnostics: Math.max(0, known.length - 20) + Math.max(0, unexpected.length - 20),
  };
  process.stdout.write(JSON.stringify(summary, null, 2) + '\n');
  if (unexpected.length !== 0) process.exitCode = 1;
}

function isKnownUnicodeDifference(test, field, expected, actual) {
  if (!test.unicode || field !== 'foldedMessage') return false;
  const projected = structuredClone(expected);
  if (test.unicode.kind === 'UTF-16 metadata property enumeration') {
    projected.metadata = test.unicode.scalarMetadata;
  } else if (test.unicode.kind === 'partial escaped lone surrogate') {
    if (projected.parts?.[0]?.input?.x !== 'hi\ud83d') return false;
    delete projected.parts[0].input;
  } else {
    return false;
  }
  return isDeepStrictEqual(projected, actual);
}

function buildCases() {
  const cases = [];
  const event = (type, values = {}) => ({ type, ...values });
  const start = (id = 'c', dynamic = false, extra = {}) =>
    event('tool-input-start', { toolCallId: id, toolName: 'code', dynamic, ...extra });
  const delta = (text = '{"x":1', id = 'c') =>
    event('tool-input-delta', { toolCallId: id, inputTextDelta: text });
  const available = (id = 'c', dynamic = false, extra = {}) =>
    event('tool-input-available', { toolCallId: id, toolName: 'code', dynamic, input: { x: 1 }, ...extra });
  const output = (id = 'c', extra = {}) =>
    event('tool-output-available', { toolCallId: id, output: 'ok', ...extra });
  const approval = (approvalId = 'a', toolCallId = 'c', extra = {}) =>
    event('tool-approval-request', { approvalId, toolCallId, ...extra });
  const response = (approvalId = 'a', extra = {}) =>
    event('tool-approval-response', { approvalId, approved: true, ...extra });
  const boundary = event('start-step');
  const tool = (toolCallId, dynamic = false, extra = {}) => ({
    type: dynamic ? 'dynamic-tool' : 'tool-code',
    ...(dynamic ? { toolName: 'code' } : {}),
    toolCallId, state: 'input-available', input: {}, ...extra,
  });
  const initial = parts => ({ id: 'm', role: 'assistant', parts });
  const add = (name, chunks, initialMessage, unicode) => cases.push({
    name,
    unicode,
    request: {
      raw_utf8: chunks.map(chunk => 'data: ' + JSON.stringify(chunk) + '\n\n').join(''),
      ...(initialMessage ? { initialMessage } : {}),
      mode: 'snapshot',
    },
  });

  for (const dynamic of [false, true]) {
    const prefix = [start('c', dynamic, {
      title: 'one', toolMetadata: { badge: 'a' }, providerMetadata: { p: { a: 1 } },
    }), delta()];
    const transitions = [
      ['available', [available('c', dynamic, { title: 'two', toolMetadata: { badge: 'b' } })]],
      ['result', [output()]],
      ['approval', [approval()]],
      ['denied', [approval(), response('a', { approved: false }), event('tool-output-denied', { toolCallId: 'c' })]],
      ['input-error', [event('tool-input-error', {
        toolCallId: 'c', toolName: 'changed', input: 'bad', dynamic: !dynamic,
        errorText: 'bad', title: 'ignored', providerMetadata: { p: { b: 2 } },
      })]],
      ['output-error', [event('tool-output-error', { toolCallId: 'c', errorText: 'failed' })]],
      ['other-step', [boundary]],
      ['other-step-available', [boundary, available('c', dynamic)]],
    ];
    for (const [name, chunks] of transitions) {
      add(`tool-transition-${dynamic}-${name}`, [...prefix, ...chunks]);
      add(`tool-late-delta-${dynamic}-${name}`, [...prefix, ...chunks, delta(',"y":2}')]);
    }
    add(`duplicate-start-${dynamic}`, [...prefix, start('c', dynamic), delta('[1')]);
    add(`duplicate-start-nextstep-${dynamic}`, [...prefix, boundary, start('c', dynamic), delta('[1')]);
    add(`duplicate-kind-${dynamic}`, [...prefix, available('c', !dynamic), output()]);
    add(`next-step-error-${dynamic}`, [...prefix, boundary, event('tool-output-error', {
      toolCallId: 'c', errorText: 'failed', providerMetadata: { p: { result: 2 } },
    })]);
  }

  const duplicatedTools = [
    [tool('c'), tool('c', true)],
    [tool('c', true), tool('c')],
    [tool('c'), tool('c', false, { input: { second: true } })],
    [tool('c'), { type: 'step-start' }, tool('c', true)],
    [tool('c', true), { type: 'step-start' }, tool('c')],
  ];
  duplicatedTools.forEach((parts, index) => {
    add(`initial-output-duplicate-layout-${index}`, [output()], initial(parts));
    add(`initial-approval-duplicate-layout-${index}`, [approval(), response()], initial(parts));
  });
  const streamedTools = [
    [tool('c', false, { state: 'input-streaming', rawInput: '{"old":', input: {}, inputEncoding: 'json' })],
    [tool('c', true, { state: 'input-streaming', rawInput: '{"old":', input: {} })],
    [
      tool('c', false, { state: 'input-streaming', rawInput: '{"first":', input: {} }),
      tool('c', false, { state: 'input-streaming', rawInput: '{"second":', input: {} }),
    ],
  ];
  streamedTools.forEach((parts, index) => add(`initial-delta-layout-${index}`, [delta('1}')], initial(parts)));
  add('approval-first-duplicate-id', [available('c'), available('d'), approval('same', 'c'), approval('same', 'd'), response('same', { approved: false }), output('c'), output('d')]);
  add('approval-id-replaced', [available('c'), available('d'), approval('same', 'c'), approval('same', 'd'), approval('new', 'c'), response('same', { approved: false })]);
  add('approval-repeat-response', [available(), approval(), response('a', { reason: 'one' }), response('a', { reason: 'two', approved: false }), response()]);

  const metadata = [null, {}, [], [1, 2], false, 0, '', 'ab', '🙂', { nested: { a: 1 }, arr: [1] }];
  for (let left = 0; left < metadata.length; left += 1) {
    for (let right = 0; right < metadata.length; right += 1) {
      const a = metadata[left];
      const b = metadata[right];
      const nonBmpSpread = (a === '🙂' && [1, 2, 4, 5, 6].includes(right))
        || (b === '🙂' && [1, 2, 3, 9].includes(left));
      const unicode = nonBmpSpread ? {
        kind: 'UTF-16 metadata property enumeration',
        scalarMetadata: { ...scalarProperties(a), ...scalarProperties(b) },
      } : undefined;
      add(`metadata-pair-${left}-${right}`, [
        event('start', { messageMetadata: a }), event('message-metadata', { messageMetadata: b }),
      ], undefined, unicode);
    }
  }
  metadata.forEach((value, index) => add(`initial-null-metadata-${index}`,
    [event('message-metadata', { messageMetadata: value })], { ...initial([]), metadata: null }));

  const inputs = [undefined, null, {}, 0, false, 'x'];
  const outputs = [undefined, null, {}, 0, false, 'y'];
  for (let left = 0; left < inputs.length; left += 1) {
    for (let right = 0; right < outputs.length; right += 1) {
      const a = inputs[left];
      const b = outputs[right];
      add(`payload-presence-${left}-${right}`, [
        event('tool-input-available', { toolName: 'code', toolCallId: 'c', ...(a !== undefined ? { input: a } : {}) }),
        event('tool-output-available', { toolCallId: 'c', ...(b !== undefined ? { output: b } : {}) }),
      ]);
      add(`data-presence-${left}-${right}`, [
        event('data-x', { id: 'd', ...(a !== undefined ? { data: a } : {}), transient: false, unknown: 1 }),
        event('data-x', { id: 'd', ...(b !== undefined ? { data: b } : {}), unknown: 2 }),
      ]);
    }
  }
  [
    JSON.parse('{"__proto__":{"polluted":true}}'),
    { constructor: { prototype: { polluted: true } } },
    { constructor: 1 }, { prototype: 2 },
  ].forEach((data, index) => add(`prototype-key-shape-${index}`, [event('data-x', { data })]));
  [
    '{"x":"hi\\', '{"x":"hi\\u12', '{"x":"hi\\ud83d', '{"x":"hi\\ud83d\\ude42',
    '{"x":[-', '{"x":[1e+', '{"x":[fa', '{"x":[nu', '{"x":tru', '{"x":1,"y":',
    '[1,2,', '-', 't', '"🙂', '{"x":"é🙂',
  ].forEach((text, index) => add(`partial-preview-${index}`, [start(), delta(text)], undefined,
    index === 2 ? { kind: 'partial escaped lone surrogate' } : undefined));
  if (cases.length !== 257) throw new Error('Differential case count changed');
  return cases;
}

function scalarProperties(value) {
  if (typeof value === 'string') return Object.fromEntries([...value].map((unit, index) => [index, unit]));
  if (value !== null && typeof value === 'object') return { ...value };
  return {};
}
