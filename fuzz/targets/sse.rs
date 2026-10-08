//! Target `sse`: the shared Server-Sent Events decoder (`hoocode-ai-sse`).
//!
//! Must not panic on any bytes, and must decode the same events however the body
//! is split into chunks (a CRLF or a UTF-8 sequence cut across two reads is the
//! classic failure).

use futures_util::StreamExt;
use hoocode_ai_sse::sse_events;

pub fn run(data: &[u8]) {
    let whole = decode(vec![data.to_vec()]);
    let step = 1 + usize::from(data.first().copied().unwrap_or(0)) % 16;
    let chunks: Vec<Vec<u8>> = data.chunks(step).map(<[u8]>::to_vec).collect();
    assert_eq!(whole, decode(chunks), "events depend on chunk boundaries");
}

/// Every item as text: events via `Debug`, errors via `Display`.
fn decode(chunks: Vec<Vec<u8>>) -> Vec<String> {
    let body =
        futures_util::stream::iter(chunks.into_iter().map(Ok::<_, std::convert::Infallible>));
    futures_executor::block_on(sse_events(body).collect::<Vec<_>>())
        .into_iter()
        .map(|item| match item {
            Ok(event) => format!("{event:?}"),
            Err(error) => format!("error: {error}"),
        })
        .collect()
}
