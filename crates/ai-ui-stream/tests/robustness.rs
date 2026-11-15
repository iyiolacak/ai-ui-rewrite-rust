//! The decoders read untrusted network bytes. Two properties, over the audit's 96 vectors as a seed corpus:
//!
//! 1. **Fragmentation invariance.** However the bytes are split into reads, the outcome (the last message and whether
//!    the stream failed) is the same.
//! 2. **No panics, bounded output.** Thousands of deterministic mutations of the corpus (flips, deletions, duplicated
//!    ranges, inserted bytes, truncation), fed to every decoder with random read boundaries, never panic and never
//!    produce an unbounded message.
//!
//! A failure prints the seed vector, the mutation and the partition so it can be turned into a fixed test.

use ai_ui_core::adapter::RunStream;
use ai_ui_core::*;
use ai_ui_stream::{
    run_stream, DataStreamDecoder, MessageAccumulator, SseDecoder, SseLimits,
    UiMessageStreamDecoder,
};
use futures_core::Stream;
use serde_json::Value;
use std::pin::Pin;
use std::task::{Context, Poll};

struct Reads(Vec<Vec<u8>>);
impl Stream for Reads {
    type Item = Result<Vec<u8>, String>;
    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready((!self.0.is_empty()).then(|| Ok(self.0.remove(0))))
    }
}

#[derive(Clone, Copy, Debug)]
enum Decoder {
    Sse,
    Data,
    Ui,
}

/// What a stream ends as: its last message and whether it failed.
#[derive(Debug, PartialEq)]
struct Outcome {
    last: Option<ChatModelRunResult>,
    failed: bool,
}

fn decode(decoder: Decoder, reads: Vec<Vec<u8>>) -> Outcome {
    let acc = MessageAccumulator::default();
    let body = Reads(reads);
    let mut stream: RunStream = match decoder {
        Decoder::Sse => run_stream(body, SseDecoder::default(), acc),
        Decoder::Data => run_stream(body, DataStreamDecoder::new(1 << 20), acc),
        Decoder::Ui => run_stream(body, UiMessageStreamDecoder::new(SseLimits::default()), acc),
    };
    let mut outcome = Outcome {
        last: None,
        failed: false,
    };
    futures_executor::block_on(async {
        while let Some(item) = std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
            match item {
                Ok(result) => outcome.last = Some(result),
                Err(_) => outcome.failed = true,
            }
        }
    });
    outcome
}

/// A small deterministic generator (xorshift64*), so failures reproduce.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn partition(bytes: &[u8], rng: &mut Rng) -> Vec<Vec<u8>> {
    let mut reads = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let len = 1 + rng.below(1 + bytes.len().min(97));
        let end = (at + len).min(bytes.len());
        reads.push(bytes[at..end].to_vec());
        at = end;
    }
    reads
}

fn mutate(bytes: &[u8], rng: &mut Rng) -> (Vec<u8>, String) {
    let mut out = bytes.to_vec();
    if out.is_empty() {
        return (out, "empty".into());
    }
    let description = match rng.below(6) {
        0 => {
            let i = rng.below(out.len());
            out[i] ^= 1 << rng.below(8);
            format!("flip bit at {i}")
        }
        1 => {
            let i = rng.below(out.len());
            let n = 1 + rng.below(16);
            out.drain(i..(i + n).min(out.len()));
            format!("delete {n} at {i}")
        }
        2 => {
            let i = rng.below(out.len());
            let n = 1 + rng.below(64);
            let copy: Vec<u8> = out[i..(i + n).min(out.len())].to_vec();
            let at = rng.below(out.len());
            out.splice(at..at, copy);
            format!("duplicate {n} from {i} to {at}")
        }
        3 => {
            let at = rng.below(out.len());
            let junk: Vec<u8> = (0..1 + rng.below(8)).map(|_| rng.next() as u8).collect();
            out.splice(at..at, junk);
            format!("insert junk at {at}")
        }
        4 => {
            let n = rng.below(out.len());
            out.truncate(n);
            format!("truncate to {n}")
        }
        _ => {
            // Structural bytes are where decoders branch.
            let specials = *b"\n\r:{}[]\",0123456789\\\0\xff";
            let at = rng.below(out.len());
            out[at] = specials[rng.below(specials.len())];
            format!("set special byte at {at}")
        }
    };
    (out, description)
}

fn corpus() -> Vec<(String, Decoder, Vec<u8>)> {
    let file: Value = serde_json::from_str(include_str!("fixtures/protocol-vectors.json")).unwrap();
    file["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|vector| {
            let decoder = match vector["protocol"].as_str().unwrap() {
                "assistant-transport" => Decoder::Sse,
                "ui-message-stream" | "ai-sdk-ui-message-stream" => Decoder::Ui,
                _ => Decoder::Data,
            };
            (
                vector["id"].as_str().unwrap().to_owned(),
                decoder,
                vector["raw_utf8"].as_str().unwrap().as_bytes().to_vec(),
            )
        })
        .collect()
}

#[test]
fn the_outcome_does_not_depend_on_how_bytes_are_split_into_reads() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut problems = Vec::new();
    for (id, decoder, bytes) in corpus() {
        let whole = decode(decoder, vec![bytes.clone()]);
        let byte_by_byte = decode(decoder, bytes.iter().map(|b| vec![*b]).collect());
        if byte_by_byte != whole {
            problems.push(format!("{id}: one byte per read differs from one read"));
        }
        for round in 0..8 {
            let reads = partition(&bytes, &mut rng);
            let shapes: Vec<usize> = reads.iter().map(Vec::len).collect();
            if decode(decoder, reads) != whole {
                problems.push(format!("{id}: partition {round} {shapes:?} differs"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn mutated_input_never_panics_and_output_stays_bounded() {
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    let decoders = [Decoder::Sse, Decoder::Data, Decoder::Ui];
    let mut runs = 0usize;
    // `AI_UI_FUZZ_ROUNDS=2000 cargo test --release ...` runs a much deeper sweep.
    let rounds: usize = std::env::var("AI_UI_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    for (id, _, bytes) in corpus() {
        for round in 0..rounds {
            let (mutated, what) = mutate(&bytes, &mut rng);
            for decoder in decoders {
                let reads = partition(&mutated, &mut rng);
                let outcome =
                    std::panic::catch_unwind(|| decode(decoder, reads)).unwrap_or_else(|_| {
                        panic!("panic: vector {id}, round {round}, {what}, {decoder:?}")
                    });
                runs += 1;
                if let Some(result) = outcome.last {
                    // A message cannot hold more than was fed to the decoder (plus bookkeeping).
                    let size = serde_json::to_string(&result.content).map_or(0, |s| s.len());
                    assert!(
                        size <= 8 * mutated.len() + 1024,
                        "unbounded output ({size} bytes from {} bytes): vector {id}, {what}, {decoder:?}",
                        mutated.len()
                    );
                }
            }
        }
    }
    println!("{runs} mutated decodes, no panic");
}
