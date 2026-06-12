/// concurrency — WS handler under parallel message load.
mod support;
///
/// Verifies stream state, emotion state, and diversifiers don't corrupt
/// under concurrent access.
///
/// cargo test -p depecherd --test concurrency -- --nocapture
use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

async fn start() -> u16 {
    std::env::set_var("DEPECHER_DRY_RUN", "1");
    std::env::remove_var("FISH_SPEECH_URL");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = depecherd::build_router().await;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

async fn ws(
    port: u16,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (ws, _) = connect_async(format!("ws://127.0.0.1:{port}/ws"))
        .await
        .unwrap();
    ws
}

async fn recv_timeout(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    ms: u64,
) -> Option<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&t) {
                    if support::is_infrastructure_msg(&msg) {
                        continue;
                    }
                    return Some(msg);
                }
            }
            _ => return None,
        }
    }
}

#[tokio::test]
async fn multiple_connections_independent() {
    let port = start().await;
    let mut ws1 = ws(port).await;
    let mut ws2 = ws(port).await;

    // Send different text on each connection
    ws1.send(Message::Text(
        json!({"type": "delta", "text": "First connection. "})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    ws2.send(Message::Text(
        json!({"type": "delta", "text": "Second connection. "})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    let msg1 = recv_timeout(&mut ws1, 1000).await;
    let msg2 = recv_timeout(&mut ws2, 1000).await;

    assert!(msg1.is_some(), "ws1 should get a response");
    assert!(msg2.is_some(), "ws2 should get a response");

    let t1 = msg1.unwrap()["text"].as_str().unwrap_or("").to_string();
    let t2 = msg2.unwrap()["text"].as_str().unwrap_or("").to_string();

    assert!(t1.contains("First"), "ws1 got: {t1}");
    assert!(t2.contains("Second"), "ws2 got: {t2}");
    eprintln!("  [concurrency] independent connections: ws1={t1:?} ws2={t2:?}");
}

#[tokio::test]
async fn emotion_on_one_connection_doesnt_leak() {
    let port = start().await;
    let mut ws1 = ws(port).await;
    let mut ws2 = ws(port).await;

    // Make ws1 angry
    ws1.send(Message::Text(
        json!({"type": "user_message", "text": "WHAT THE HELL!!"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let e1 = recv_timeout(&mut ws1, 1000).await.unwrap();
    assert_eq!(e1["emotion"], "angry");

    // ws2 should be neutral (separate emotion state)
    ws2.send(Message::Text(
        json!({"type": "user_message", "text": "Refactor the config module"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let e2 = recv_timeout(&mut ws2, 1000).await.unwrap();
    assert_eq!(
        e2["emotion"], "neutral",
        "emotion leaked between connections"
    );

    eprintln!("  [concurrency] emotion isolation: ws1=angry ws2=neutral ✅");
}

#[tokio::test]
async fn rapid_deltas_dont_corrupt_stream() {
    let port = start().await;
    let mut ws1 = ws(port).await;

    // Send 50 rapid deltas without waiting for responses
    for i in 0..50 {
        let text = if i % 10 == 9 {
            format!("Word{i}. ")
        } else {
            format!("Word{i} ")
        };
        ws1.send(Message::Text(
            json!({"type": "delta", "text": text}).to_string().into(),
        ))
        .await
        .unwrap();
    }

    // Flush
    ws1.send(Message::Text(
        json!({"type": "message_end"}).to_string().into(),
    ))
    .await
    .unwrap();

    // Collect all responses
    let mut responses = Vec::new();
    for _ in 0..20 {
        match recv_timeout(&mut ws1, 500).await {
            Some(msg) if msg["type"] == "speak" => responses.push(msg),
            _ => break,
        }
    }

    assert!(
        !responses.is_empty(),
        "should produce speak messages from rapid deltas"
    );
    eprintln!(
        "  [concurrency] 50 rapid deltas → {} speak messages",
        responses.len()
    );
}

#[tokio::test]
async fn reset_during_buffering() {
    let port = start().await;
    let mut ws1 = ws(port).await;

    // Start buffering text
    ws1.send(Message::Text(
        json!({"type": "delta", "text": "This is buffered"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    // Reset mid-buffer
    ws1.send(Message::Text(json!({"type": "reset"}).to_string().into()))
        .await
        .unwrap();

    // Flush — should have nothing since reset cleared the buffer
    ws1.send(Message::Text(
        json!({"type": "message_end"}).to_string().into(),
    ))
    .await
    .unwrap();

    let msg = recv_timeout(&mut ws1, 500).await;
    assert!(
        msg.is_none(),
        "reset should clear the buffer, nothing to flush"
    );
    eprintln!("  [concurrency] reset during buffering: buffer cleared ✅");
}

#[tokio::test]
async fn interleaved_emotion_and_delta() {
    let port = start().await;
    let mut ws1 = ws(port).await;

    // Interleave emotion and delta messages rapidly
    ws1.send(Message::Text(
        json!({"type": "user_message", "text": "This is amazing!!!"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    ws1.send(Message::Text(
        json!({"type": "delta", "text": "Hello world. "})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    ws1.send(Message::Text(
        json!({"type": "user_message", "text": "ugh, not again..."})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    ws1.send(Message::Text(
        json!({"type": "delta", "text": "More text. "})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    // Collect all responses
    let mut emotions = Vec::new();
    let mut speaks = Vec::new();
    for _ in 0..10 {
        match recv_timeout(&mut ws1, 500).await {
            Some(msg) if msg["type"] == "emotion" => emotions.push(msg),
            Some(msg) if msg["type"] == "speak" => speaks.push(msg),
            _ => break,
        }
    }

    assert!(!emotions.is_empty(), "should get emotion responses");
    assert!(!speaks.is_empty(), "should get speak responses");
    eprintln!(
        "  [concurrency] interleaved: {} emotions + {} speaks",
        emotions.len(),
        speaks.len()
    );
}

#[tokio::test]
async fn ten_connections_simultaneous() {
    let port = start().await;

    let mut handles = Vec::new();
    for i in 0..10 {
        let p = port;
        handles.push(tokio::spawn(async move {
            let mut ws = ws(p).await;
            ws.send(Message::Text(
                json!({"type": "delta", "text": format!("Conn{i} sentence. ")})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            let msg = recv_timeout(&mut ws, 2000).await;
            msg.is_some()
        }));
    }

    let results: Vec<bool> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap_or(false))
        .collect();

    let success_count = results.iter().filter(|&&b| b).count();
    eprintln!("  [concurrency] 10 simultaneous connections: {success_count}/10 got responses");
    assert!(
        success_count >= 8,
        "at least 8/10 should succeed: {success_count}"
    );
}
