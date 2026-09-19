#![cfg(feature = "fake")]

use std::time::Duration;

use arkd_core::playtools::fake::FakePlayToolsServer;
use arkd_core::playtools::{Frame, PlayToolsClient};

async fn wait_touches(server: &FakePlayToolsServer, n: usize) {
    for _ in 0..100 {
        if server.touches.lock().unwrap().len() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn frame(w: u32, h: u32) -> Frame {
    let mut bgr = Vec::with_capacity((w * h * 3) as usize);
    for i in 0..(w * h) {
        bgr.extend_from_slice(&[(i % 256) as u8, 100, 200]);
    }
    Frame {
        width: w,
        height: h,
        bgr,
    }
}

#[tokio::test]
async fn handshake_version_size_bundle() {
    let server = FakePlayToolsServer::spawn(frame(16, 8), 3).await;
    let mut client = PlayToolsClient::connect(&server.addr.to_string(), Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(client.version(), 3);
    assert_eq!(client.size(), (16, 8));
    assert_eq!(
        client.bundle_id().await.unwrap(),
        "com.hypergryph.arknights"
    );
}

#[tokio::test]
async fn capture_uses_bgr_when_version_at_least_3() {
    let server = FakePlayToolsServer::spawn(frame(16, 8), 4).await;
    let mut client = PlayToolsClient::connect(&server.addr.to_string(), Duration::from_secs(5))
        .await
        .unwrap();
    let f = client.capture().await.unwrap();
    assert_eq!((f.width, f.height), (16, 8));
    assert_eq!(f.bgr.len(), 16 * 8 * 3);
    assert_eq!(&f.bgr[..3], &[0, 100, 200]);
}

#[tokio::test]
async fn capture_uses_rgba_when_version_below_3() {
    let server = FakePlayToolsServer::spawn(frame(16, 8), 2).await;
    let mut client = PlayToolsClient::connect(&server.addr.to_string(), Duration::from_secs(5))
        .await
        .unwrap();
    let f = client.capture().await.unwrap();
    assert_eq!((f.width, f.height), (16, 8));
    assert_eq!(f.bgr.len(), 16 * 8 * 3);
    assert_eq!(&f.bgr[..3], &[0, 100, 200]);
}

#[tokio::test]
async fn tap_records_began_and_ended() {
    let server = FakePlayToolsServer::spawn(frame(16, 8), 3).await;
    let mut client = PlayToolsClient::connect(&server.addr.to_string(), Duration::from_secs(5))
        .await
        .unwrap();
    client
        .tap(100, 200, Duration::from_millis(10))
        .await
        .unwrap();
    wait_touches(&server, 2).await;
    let touches = server.touches.lock().unwrap();
    let phases: Vec<u8> = touches.iter().map(|t| t.phase).collect();
    assert_eq!(phases, vec![0, 3]);
    assert_eq!((touches[0].x, touches[0].y), (100, 200));
}

#[tokio::test]
async fn drag_records_moved_between_endpoints() {
    let server = FakePlayToolsServer::spawn(frame(16, 8), 3).await;
    let mut client = PlayToolsClient::connect(&server.addr.to_string(), Duration::from_secs(5))
        .await
        .unwrap();
    client
        .drag(
            &[(10, 10), (50, 50), (90, 90)],
            Duration::from_millis(5),
            Duration::from_millis(5),
        )
        .await
        .unwrap();
    wait_touches(&server, 4).await;
    let touches = server.touches.lock().unwrap();
    let phases: Vec<u8> = touches.iter().map(|t| t.phase).collect();
    assert_eq!(phases, vec![0, 1, 1, 3]);
    assert_eq!(
        (touches.last().unwrap().x, touches.last().unwrap().y),
        (90, 90)
    );
}

#[tokio::test]
async fn png_round_trip_dimensions() {
    let f = frame(20, 10);
    let png = f.to_png().unwrap();
    let dims = arkd_core::screen::png_dimensions(&png).unwrap();
    assert_eq!(dims, (20, 10));
}

#[tokio::test]
async fn handshake_refusal_is_reported() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = listener.accept().await {
            use tokio::io::AsyncWriteExt;
            s.write_all(b"NOPE").await.ok();
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
    let err = PlayToolsClient::connect(&addr.to_string(), Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("handshake refused"), "{err}");
}
