//! Drives the client against a scripted NT4 server on localhost.

use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use nt4::{Client, Value};
use serde_json::json;
use tungstenite::handshake::server::{Request, Response};
use tungstenite::Message;

fn frame(id: i64, ts: i64, type_id: u8, value: rmpv::Value) -> Vec<u8> {
    let mut buf = Vec::new();
    rmpv::encode::write_value(&mut buf, &rmpv::Value::Array(vec![id.into(), ts.into(), type_id.into(), value])).unwrap();
    buf
}

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn subscribes_decodes_and_recovers() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut ws = tungstenite::accept_hdr(stream, |req: &Request, mut resp: Response| {
            let offered = req.headers().get("Sec-WebSocket-Protocol").unwrap().to_str().unwrap().to_owned();
            assert!(offered.contains("v4.1.networktables.first.wpi.edu"), "{offered}");
            assert_eq!(req.uri().path(), "/nt/catalyst-linux-test");
            resp.headers_mut().insert("Sec-WebSocket-Protocol", "v4.1.networktables.first.wpi.edu".parse().unwrap());
            Ok(resp)
        })
        .unwrap();

        // The first thing the client says is its subscription.
        let first = ws.read().unwrap();
        let sub: serde_json::Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
        assert_eq!(sub[0]["method"], "subscribe");
        assert_eq!(sub[0]["params"]["topics"], json!(["/Catalyst/"]));
        assert_eq!(sub[0]["params"]["options"]["prefix"], true);

        let announce = json!([
            {"method": "announce", "params": {"name": "/Catalyst/Systemcore/BatteryVolts", "id": 7, "type": "double", "properties": {}}},
            {"method": "announce", "params": {"name": "/Catalyst/Health/History", "id": 8, "type": "string[]", "properties": {}}},
            {"method": "announce", "params": {"name": "/Catalyst/Health/Healthy", "id": 9, "type": "boolean", "properties": {}}},
            {"method": "announce", "params": {"name": "/Catalyst/Health/ErrorCount", "id": 10, "type": "int", "properties": {}}},
            {"method": "announce", "params": {"name": "/Catalyst/Gone", "id": 11, "type": "double", "properties": {}}}
        ]);
        ws.send(Message::text(announce.to_string())).unwrap();

        // Several value frames in one WebSocket message.
        let mut batch = frame(7, 1000, 1, 12.4.into());
        batch.extend(frame(8, 1001, 20, rmpv::Value::Array(vec!["1.500|FIRED|WARN|Elevator|HighTemp|71 C".into()])));
        batch.extend(frame(9, 1002, 0, false.into()));
        batch.extend(frame(10, 1003, 2, 2.into()));
        ws.send(Message::binary(batch)).unwrap();
        ws.send(Message::text(json!([{"method": "unannounce", "params": {"name": "/Catalyst/Gone", "id": 11}}]).to_string())).unwrap();

        // Answer one time sync, then hang up.
        loop {
            let msg = ws.read().unwrap();
            if let Message::Binary(bytes) = msg {
                let rmpv::Value::Array(f) = rmpv::decode::read_value(&mut &bytes[..]).unwrap() else { panic!() };
                assert_eq!(f[0].as_i64(), Some(-1));
                ws.send(Message::binary(frame(-1, 5_000_000, 2, f[3].clone()))).unwrap();
                break;
            }
        }
        thread::sleep(Duration::from_millis(300));
        ws.close(None).ok();
        let _ = ws.read();
    });

    let client = Client::connect(&format!("127.0.0.1:{port}"), "catalyst-linux-test", &["/Catalyst/"]);
    wait_for("battery value", || client.f64("/Catalyst/Systemcore/BatteryVolts") == Some(12.4));
    assert!(client.connected());
    assert_eq!(client.status(), "connected");
    wait_for("the rest of the batch", || client.i64("/Catalyst/Health/ErrorCount") == Some(2));
    assert_eq!(client.bool("/Catalyst/Health/Healthy"), Some(false));
    assert_eq!(
        client.get("/Catalyst/Health/History"),
        Some(Value::StringArray(vec!["1.500|FIRED|WARN|Elevator|HighTemp|71 C".into()]))
    );
    wait_for("unannounce", || client.topic_count() == 4);
    assert_eq!(client.with_prefix("/Catalyst/Health/").len(), 3);
    // Absent means absent.
    assert_eq!(client.f64("/Catalyst/Systemcore/CpuPercent"), None);
    wait_for("round trip time", || client.rtt_ms().is_some());

    // The server hangs up: the client says so and forgets stale values.
    wait_for("disconnect", || !client.connected());
    assert_eq!(client.f64("/Catalyst/Systemcore/BatteryVolts"), None);
    assert_ne!(client.status(), "connected");
    server.join().unwrap();
}
