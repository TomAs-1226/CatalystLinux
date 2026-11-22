//! `catalyst-companiond`: the handheld's side of Catalyst Companion, the
//! app for a Windows computer.
//!
//! It answers "is a handheld there?" on UDP, and serves HTTPS (TLS 1.3, its
//! own pinned certificate) on every network the handheld is on: Wi-Fi and
//! the USB cable (192.168.7.1). A computer pairs once with the six-digit
//! code Settings › Computer companion shows, and is given a token; every
//! request after that carries it. Both threads sleep in the kernel until
//! something arrives, at a low priority: nothing is spent while no computer
//! asks. Off in Settings, the service is not started at all.
//!
//!     catalyst-companiond [--data /data] [--run /run/catalyst] [--port 47315] [--bind 0.0.0.0]
//!
//! `CATALYST_COMPANION_SHELL=127.0.0.1:PORT` reaches a simulator's shell
//! on a computer instead of the handheld's socket.

mod api;
mod files;
mod http;
mod shell;
mod store;
mod tls;

use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use catalyst_companion_proto as proto;

/// Connections looked after at once: a computer uploading a few songs side
/// by side, and its status asked meanwhile.
const AT_ONCE: usize = 6;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let on_device = cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() && value("--data").is_none();
    let paths = store::Paths { data: value("--data").unwrap_or_else(|| "/data".into()).into(), run: value("--run").unwrap_or_else(|| "/run/catalyst".into()).into() };
    let port: u16 = value("--port").and_then(|p| p.parse().ok()).unwrap_or(proto::PORT);
    // Every network by default; a computer trying it on its own takes 127.0.0.1.
    let on: std::net::IpAddr = value("--bind").and_then(|b| b.parse().ok()).unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    #[cfg(unix)]
    // SAFETY: setpriority on this process only.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
    std::fs::create_dir_all(paths.own()).ok();
    std::fs::create_dir_all(&paths.run).ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(paths.own(), std::fs::Permissions::from_mode(0o700)).ok();
    }
    let shell = shell::Socket { path: paths.shell_socket(), tcp: std::env::var("CATALYST_COMPANION_SHELL").ok().filter(|s| !s.is_empty()) };
    let ctx = Arc::new(api::Ctx {
        paths: paths.clone(),
        shell: Box::new(shell),
        pairing: Default::default(),
        on_device,
        updater: "/usr/bin/catalyst-update".into(),
        penalty: Duration::from_millis(400),
    });
    let listener = match TcpListener::bind((on, port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("catalyst-companiond: could not listen on port {port}: {e}");
            std::process::exit(1);
        }
    };
    let answering = paths.clone();
    std::thread::Builder::new().name("discovery".into()).stack_size(128 * 1024).spawn(move || discovery(&answering, on, port)).ok();
    let tls = Arc::new(tls::Tls::new());
    let open = Arc::new(AtomicUsize::new(0));
    eprintln!("catalyst-companiond: listening on port {port}");
    for stream in listener.incoming().flatten() {
        if open.load(Ordering::SeqCst) >= AT_ONCE {
            continue;
        }
        open.fetch_add(1, Ordering::SeqCst);
        let (ctx, tls, done) = (ctx.clone(), tls.clone(), open.clone());
        let spawned = std::thread::Builder::new().name("request".into()).stack_size(512 * 1024).spawn(move || {
            connection(&ctx, &tls, stream);
            done.fetch_sub(1, Ordering::SeqCst);
        });
        if spawned.is_err() {
            open.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// One connection: TLS, one request, its answer.
fn connection(ctx: &api::Ctx, tls: &tls::Tls, tcp: TcpStream) {
    tcp.set_read_timeout(Some(Duration::from_secs(30))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(60))).ok();
    tcp.set_nodelay(true).ok();
    let Ok((config, fingerprint)) = tls.config(&ctx.paths).map_err(|e| eprintln!("catalyst-companiond: {e}")) else { return };
    let Ok(conn) = rustls::ServerConnection::new(config) else { return };
    let mut stream = rustls::StreamOwned::new(conn, tcp);
    let after = {
        let mut reader = BufReader::with_capacity(16 * 1024, &mut stream);
        let request = match http::read_request(&mut reader) {
            Ok(Some(r)) => r,
            _ => return,
        };
        let response = {
            let mut body = std::io::Read::take(&mut reader, request.length);
            api::handle(ctx, &request, &mut body, &fingerprint)
        };
        // What it asked and how it went; never what it carried.
        if response.status >= 400 && response.status != 401 {
            eprintln!("catalyst-companiond: {} {} → {}", request.method, request.path, response.status);
        }
        match http::write_response(reader.get_mut(), response) {
            Ok(after) => after,
            Err(_) => return,
        }
    };
    stream.conn.send_close_notify();
    stream.flush().ok();
    if let Some(after) = after {
        after();
    }
}

/// Answers "is a handheld there?", from any network, with who this is.
fn discovery(paths: &store::Paths, on: std::net::IpAddr, port: u16) {
    let Ok(socket) = UdpSocket::bind((on, proto::DISCOVERY_PORT)) else {
        eprintln!("catalyst-companiond: could not listen for computers looking");
        return;
    };
    let mut buf = [0u8; 64];
    let mut answered = std::time::Instant::now();
    let mut lately = 0u32;
    while let Ok((n, from)) = socket.recv_from(&mut buf) {
        if &buf[..n] != proto::DISCOVERY_ASK {
            continue;
        }
        // A few answers a second at most, however many ask.
        if answered.elapsed() > Duration::from_secs(1) {
            answered = std::time::Instant::now();
            lately = 0;
        }
        lately += 1;
        if lately > 20 {
            continue;
        }
        let reply = serde_json::json!({"catalyst": 1, "id": store::id(paths), "name": store::name(), "version": store::version(), "port": port});
        socket.send_to(reply.to_string().as_bytes(), from).ok();
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

    /// Takes any certificate, as the app does while pairing, and notes it.
    #[derive(Debug)]
    struct Note(Mutex<Option<Vec<u8>>>, Arc<rustls::crypto::CryptoProvider>);

    impl ServerCertVerifier for Note {
        fn verify_server_cert(&self, cert: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
            *self.0.lock().unwrap() = Some(cert.to_vec());
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &rustls::DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
            Err(rustls::Error::General("TLS 1.3 only".into()))
        }
        fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(m, c, d, &self.1.signature_verification_algorithms)
        }
        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.1.signature_verification_algorithms.supported_schemes()
        }
    }

    #[test]
    fn a_connection_is_tls_13_with_the_certificate_made_and_kept_and_answers_hello() {
        let root = std::env::temp_dir().join(format!("companiond-tls-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let paths = super::store::Paths { data: root.join("data"), run: root.join("run") };
        let ctx = Arc::new(super::api::Ctx {
            paths: paths.clone(),
            shell: Box::new(super::shell::Socket { path: root.join("none.sock"), tcp: None }),
            pairing: Default::default(),
            on_device: false,
            updater: "/nowhere".into(),
            penalty: std::time::Duration::ZERO,
        });
        let tls = Arc::new(super::tls::Tls::new());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (c, t) = (ctx.clone(), tls.clone());
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (s, _) = listener.accept().unwrap();
                super::connection(&c, &t, s);
            }
        });
        let ask = || {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let note = Arc::new(Note(Mutex::new(None), provider.clone()));
            let config = rustls::ClientConfig::builder_with_provider(provider).with_protocol_versions(&[&rustls::version::TLS13]).unwrap().dangerous().with_custom_certificate_verifier(note.clone()).with_no_client_auth();
            let conn = rustls::ClientConnection::new(Arc::new(config), ServerName::try_from("catalyst.local").unwrap()).unwrap();
            let mut s = rustls::StreamOwned::new(conn, std::net::TcpStream::connect(addr).unwrap());
            s.write_all(b"GET /v1/hello HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut reply = String::new();
            s.read_to_string(&mut reply).unwrap();
            assert_eq!(s.conn.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_3));
            let cert = note.0.lock().unwrap().clone().unwrap();
            (reply, cert)
        };
        let (reply, first) = ask();
        assert!(reply.starts_with("HTTP/1.1 200") && reply.contains("\"catalyst\":1"), "{reply}");
        assert_eq!(std::fs::read(paths.cert()).unwrap(), first, "the certificate is the one kept on /data");
        let (_, second) = ask();
        assert_eq!(first, second, "and the same one each time, for the computer to pin");
        server.join().unwrap();
    }
}
