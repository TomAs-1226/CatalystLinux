//! The installer's protocols against kflash.py's bytes and a stand-in K210,
//! USB detection against a scratch sysfs, and a whole install.

use super::isp::{self, Isp, Unslip, Wiring};
use super::repl::{self, Repl};
use super::stub::{Fake, Firmware, Port};
use super::*;
use std::sync::{Arc, Mutex};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ------------------------------------------------------------- ISP framing ---

#[test]
fn slip_escapes_as_kflash_does() {
    assert_eq!(isp::slip(&[1, 0xC0, 2, 0xDB, 3]), vec![0xC0, 1, 0xDB, 0xDC, 2, 0xDB, 0xDD, 3, 0xC0]);
    // Back again, however the bytes are cut, with junk before a frame.
    let wire = [vec![0x55, 0x66], isp::slip(&[0xC0, 0xDB, 7]), isp::slip(&[0xD2, 0xE0])].concat();
    for cut in 1..wire.len() {
        let mut u = Unslip::default();
        let got: Vec<Vec<u8>> = wire.chunks(cut).flat_map(|c| u.push(c)).collect();
        assert_eq!(got, vec![vec![0xC0, 0xDB, 7], vec![0xD2, 0xE0]], "cut {cut}");
    }
}

#[test]
fn packets_are_kflashs_byte_for_byte() {
    // Worked out with kflash.py's own struct.pack and binascii.crc32.
    assert_eq!(hex(&isp::slip(&isp::words_packet(isp::ROM_BOOT, isp::SRAM, 0))), "c0c5000000fb6fc4d40000008000000000c0");
    assert_eq!(hex(&isp::slip(&isp::words_packet(isp::FLASH_INIT, 1, 0))), "c0d7000000f7df88a90100000000000000c0");
    assert_eq!(hex(&isp::slip(&isp::baud_packet(1_500_000))), "c0d6000000b8e48573000000000400000060e31600c0");
    assert_eq!(hex(&isp::slip(&isp::write_packet(isp::ROM_WRITE, 0x8000_0400, &[0xC0, 0xDB, 1, 2]))), "c0c3000000e2df97290004008004000000dbdcdbdd0102c0");
    // The greetings, as kflash.py writes them literally.
    assert_eq!(isp::greeting(isp::ROM_NOP), [&[0xC0, 0xC2][..], &[0; 12], &[0xC0]].concat());
    assert_eq!(isp::greeting(isp::FLASH_NOP).len(), 15);
    // The checksum is zlib's CRC-32.
    assert_eq!(crate::smartcam::frame::crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn a_firmware_carries_its_length_and_sha256() {
    use sha2::Digest;
    let image = isp::firmware_image(b"maixpy");
    assert_eq!(&image[..5], &[0, 6, 0, 0, 0]);
    assert_eq!(&image[5..11], b"maixpy");
    assert_eq!(&image[11..], &sha2::Sha256::digest(&image[..11])[..]);
}

/// A stand-in K210 shared with the test, and its port.
fn fake(f: Fake) -> (Arc<Mutex<Fake>>, Port) {
    let shared = f.shared();
    (shared.clone(), Port(shared))
}

#[test]
fn the_isp_loads_its_loader_and_writes_the_flash() {
    let (k210, mut port) = fake(Fake::new(Firmware::None).running());
    let loader: Vec<u8> = (0..3000u32).map(|i| i as u8).collect();
    let firmware: Vec<u8> = (0..150_000u32).map(|i| (i * 7) as u8).collect();
    let model: Vec<u8> = (0..70_000u32).map(|i| (i * 13) as u8).collect();
    let mut isp = Isp::enter(&mut port, Wiring::Dan).expect("into the ISP");
    let mut seen = 0;
    isp.load(&loader, &mut |n| seen = n).expect("loader");
    assert_eq!(seen, loader.len());
    assert!(isp.faster(1_500_000));
    isp.init_flash().expect("flash chosen");
    let image = isp::firmware_image(&firmware);
    isp.write_flash(0, &image, &mut |_| {}).expect("firmware");
    isp.write_flash(MODEL_AT, &model, &mut |n| seen = n).expect("model");
    assert_eq!(seen, model.len());
    assert_eq!(isp.finish(), Wiring::Dan);
    let k = k210.lock().unwrap();
    assert_eq!(&k.flash[..image.len()], &image[..]);
    assert_eq!(&k.flash[MODEL_AT as usize..MODEL_AT as usize + model.len()], &model[..]);
    assert_eq!(k.firmware, Firmware::MaixPy { full: true }, "a whole image at 0 is a firmware");
    assert!(k.log.contains(&"loader".into()) && k.log.contains(&"flash 0x0".into()) && k.log.contains(&"flash 0x300000".into()), "{:?}", k.log);
}

#[test]
fn the_isp_finds_the_other_wiring() {
    let mut f = Fake::new(Firmware::None).running();
    f.wiring = Wiring::Kd233;
    let (_, mut port) = fake(f);
    let isp = Isp::enter(&mut port, Wiring::Dan).expect("found");
    assert_eq!(isp.wiring, Wiring::Kd233);
}

#[test]
fn a_bridge_that_cannot_go_fast_says_so() {
    let mut f = Fake::new(Firmware::None).running();
    f.max_baud = 921_600;
    let (_, mut port) = fake(f);
    let mut isp = Isp::enter(&mut port, Wiring::Dan).unwrap();
    isp.load(&[1, 2, 3], &mut |_| {}).unwrap();
    assert!(!isp.faster(1_500_000), "the loader moved, the bridge could not follow");
}

#[test]
fn a_corrupted_request_is_refused_and_sent_again() {
    // The stand-in checks each request's CRC as the loader does: one with a
    // bad one is answered E2 and written nowhere (`request` sends it again).
    let (k210, mut port) = fake(Fake::new(Firmware::None).running());
    let mut session = Isp::enter(&mut port, Wiring::Dan).unwrap();
    session.load(&[9; 10], &mut |_| {}).unwrap();
    drop(session);
    let mut bad = isp::write_packet(isp::FLASH_WRITE, 0, &[1, 2, 3]);
    bad[4] ^= 0xFF;
    port.write(&isp::slip(&bad));
    let mut buf = [0u8; 64];
    let n = port.read(&mut buf, Duration::from_millis(200));
    assert_eq!(Unslip::default().push(&buf[..n]), vec![vec![isp::FLASH_WRITE, 0xE2]]);
    assert!(k210.lock().unwrap().flash.is_empty());
}

// ----------------------------------------------------------------- the REPL ---

#[test]
fn bytes_literals_round_trip_every_byte() {
    let all: Vec<u8> = (0..=255).collect();
    let lit = repl::bytes_literal(&all);
    assert!(lit.is_ascii() && lit.starts_with("b'") && !lit[2..lit.len() - 1].contains("\n"));
    assert_eq!(repl::parse_bytes_literal(&lit), Some(all));
    assert_eq!(repl::bytes_literal(b"a'b\\"), r"b'a\'b\\'");
}

fn maixpy_with(paste: bool) -> (Arc<Mutex<Fake>>, Port) {
    let mut f = Fake::new(Firmware::MaixPy { full: true });
    f.paste = paste;
    f.files.insert("/flash/main.py".into(), b"while True: pass\n".to_vec());
    fake(f.running())
}

#[test]
fn the_raw_repl_writes_a_file_like_maixpys_old_micropython() {
    let (k210, mut port) = maixpy_with(false);
    let data: Vec<u8> = (0..1500u32).map(|i| (i * 31) as u8).collect();
    let mut repl = Repl::enter(&mut port, Duration::from_secs(2)).expect("raw REPL, main.py stopped");
    let mut last = 0;
    repl.write_file("/flash/main.py", &data, &mut |n| last = n).expect("written and read back");
    assert_eq!(repl.pasted(), Some(false), "no raw-paste on MaixPy: the plain way");
    assert_eq!(last, data.len());
    assert_eq!(k210.lock().unwrap().files["/flash/main.py"], data);
    // An error comes back as one.
    assert!(repl.exec("nonsense(", Duration::from_secs(1)).unwrap_err().contains("SyntaxError"));
    assert!(repl.read_file("/flash/nothing").unwrap_err().contains("ENOENT"));
    // And a later exec still works.
    assert_eq!(repl.read_file("/flash/main.py").unwrap(), data);
}

#[test]
fn the_raw_repl_pastes_where_micropython_can() {
    let (k210, mut port) = maixpy_with(true);
    let data = vec![b'x'; 700];
    let mut repl = Repl::enter(&mut port, Duration::from_secs(2)).unwrap();
    repl.write_file("/flash/boot.py", &data, &mut |_| {}).unwrap();
    assert_eq!(repl.pasted(), Some(true));
    assert_eq!(k210.lock().unwrap().files["/flash/boot.py"], data);
}

#[test]
fn nothing_answers_without_maixpy() {
    let (_, mut port) = fake(Fake::new(Firmware::None).running());
    assert!(Repl::enter(&mut port, Duration::from_millis(500)).is_err());
}

#[test]
fn a_probe_is_read() {
    let stamp = Stamp { catalyst: "0.9.1".into(), main: "aaaaaaaaaaaa".into(), model: "bbbbbbbbbbbb".into(), firmware: "x".into() };
    let out = format!("noise\r\nCATALYST|1|0.6.3|{}\r\n", stamp.json());
    assert_eq!(parse_probe(&out), Some(Found::MaixPy { full: true, version: "0.6.3".into(), stamp: Some(stamp) }));
    assert_eq!(parse_probe("CATALYST|0|0.5.0|{}"), Some(Found::MaixPy { full: false, version: "0.5.0".into(), stamp: None }));
    assert_eq!(parse_probe("Traceback"), None);
}

// -------------------------------------------------------------- detection ---

struct Sysfs(PathBuf);

impl Sysfs {
    fn new(name: &str) -> Sysfs {
        let root = std::env::temp_dir().join(format!("catalyst-unitv-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        Sysfs(root)
    }

    fn device(&self, id: &str, vid: &str, pid: &str, product: &str) -> &Sysfs {
        let d = self.0.join(id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("idVendor"), format!("{vid}\n")).unwrap();
        std::fs::write(d.join("idProduct"), format!("{pid}\n")).unwrap();
        if !product.is_empty() {
            std::fs::write(d.join("product"), format!("{product}\n")).unwrap();
        }
        self
    }

    fn tty(&self, id: &str, iface: &str, tty: &str) -> &Sysfs {
        let iface = format!("{id}{IFACE}{iface}");
        let at = if tty.starts_with("ttyACM") { self.0.join(&iface).join("tty").join(tty) } else { self.0.join(&iface).join(tty) };
        std::fs::create_dir_all(at).unwrap();
        self
    }
}

impl Drop for Sysfs {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[test]
fn a_unitv_is_noticed_on_its_ft2232() {
    let sys = Sysfs::new("ft2232");
    sys.device("1-1", "0424", "2514", "USB hub").device("1-1.2", "0403", "6010", "M5Stack").tty("1-1.2", "1.0", "ttyUSB0").tty("1-1.2", "1.1", "ttyUSB1");
    let b = scan(&sys.0).expect("found");
    assert_eq!((b.id.as_str(), b.chip, b.sure), ("1-1.2", "FT2232", true));
    assert_eq!(b.ttys, vec!["/dev/ttyUSB1", "/dev/ttyUSB0"], "the K210's UART is the second port: tried first");
}

#[test]
fn other_bridges_are_known_by_name_or_not_at_all() {
    let sys = Sysfs::new("bridges");
    // An unnamed CH340 (an Arduino, perhaps): only a serial device.
    sys.device("1-1.1", "1a86", "7523", "USB Serial").tty("1-1.1", "1.0", "ttyUSB0");
    let b = scan(&sys.0).unwrap();
    assert_eq!((b.chip, b.sure), ("CH340", false));
    // A CH9102 that names itself (cdc-acm): known, and preferred.
    sys.device("1-1.4", "1a86", "55d4", "UnitV-M12 K210").tty("1-1.4", "1.0", "ttyACM0");
    let b = scan(&sys.0).unwrap();
    assert_eq!((b.chip, b.sure, b.ttys.clone()), ("CH9102", true, vec!["/dev/ttyACM0".to_owned()]));
    let s = State { board: Some(b), found: Found::Looking, job: Job::Idle };
    assert_eq!(need(&s, None), Need::Looking);
}

#[test]
fn a_bridge_without_its_driver_is_said_so() {
    let sys = Sysfs::new("nodriver");
    sys.device("1-1.3", "0403", "6010", "");
    let b = scan(&sys.0).unwrap();
    assert!(b.ttys.is_empty());
    assert_eq!(need(&State { board: Some(b), found: Found::Looking, job: Job::Idle }, None), Need::NoDriver);
    let none = Sysfs::new("empty");
    none.device("1-1", "046d", "c52b", "Receiver");
    assert_eq!(scan(&none.0), None);
}

#[test]
fn what_is_offered_follows_the_stamp() {
    let board = Board { id: "1-1.2".into(), chip: "FT2232", name: String::new(), ttys: vec!["/dev/ttyUSB1".into()], sure: true };
    let now = Stamp { catalyst: "0.9.1".into(), main: "m1".into(), model: "k1".into(), firmware: String::new() };
    let st = |found: Found| State { board: Some(board.clone()), found, job: Job::Idle };
    assert_eq!(need(&st(Found::NoMaixPy), Some(&now)), Need::Install);
    assert_eq!(need(&st(Found::MaixPy { full: false, version: "0.6.3".into(), stamp: Some(now.clone()) }), Some(&now)), Need::Install);
    assert_eq!(need(&st(Found::MaixPy { full: true, version: "0.6.3".into(), stamp: None }), Some(&now)), Need::Install);
    let older = Stamp { main: "m0".into(), catalyst: "0.9.0".into(), ..now.clone() };
    assert_eq!(need(&st(Found::MaixPy { full: true, version: "0.6.3".into(), stamp: Some(older) }), Some(&now)), Need::Update);
    let same_but_firmware = Stamp { firmware: "other".into(), ..now.clone() };
    assert_eq!(need(&st(Found::MaixPy { full: true, version: "0.6.3".into(), stamp: Some(same_but_firmware) }), Some(&now)), Need::Current);
    assert_eq!(need(&State { board: None, found: Found::Looking, job: Job::Idle }, Some(&now)), Need::Unplugged);
}

#[test]
fn progress_runs_from_nothing_to_all() {
    let plan = Plan { firmware: true, model: true };
    let mut last = -1.0;
    for step in [Step::Firmware, Step::Model, Step::Program, Step::Check] {
        for d in [0.0, 0.5, 1.0] {
            let o = overall(step, d, plan);
            assert!(o >= last, "{step:?} {d}");
            last = o;
        }
    }
    assert_eq!(overall(Step::Firmware, 0.0, plan), 0.0);
    assert_eq!(overall(Step::Check, 1.0, plan), 1.0);
    // Nothing to flash: main.py starts at the beginning.
    assert_eq!(overall(Step::Program, 0.0, Plan::default()), 0.0);
}

// ------------------------------------------------------- a whole install ---

#[test]
fn the_hello_never_holds_a_ctrl_c() {
    // The stand-in stops main.py on any 03, as MaixPy's REPL does: answered
    // means none was sent.
    let mut f = Fake::new(Firmware::MaixPy { full: true });
    f.files.insert("/flash/main.py".into(), b"usb = UART.repl_uart()".to_vec());
    let (_, mut port) = fake(f.running());
    std::thread::sleep(Duration::from_millis(50));
    assert!(check(&mut port, &mut |_| {}));
}

#[test]
fn a_bare_unitv_gets_everything_and_answers_then_an_update_only_main_py() {
    let _one = sim::exclusive();
    let payload = Payload::load().unwrap();
    let board = sim::board_for_shots();
    let steps = Mutex::new(Vec::new());
    let report = |s: Step, d: f32, p: Plan| steps.lock().unwrap().push((s, d, p));

    // Nothing on it that answers: MaixPy, the model, main.py.
    let k210 = Fake::new(Firmware::None).running().shared();
    sim::plug_with(Some(k210.clone()));
    let (answered, plan, stamp) = install_on(&board, None, &payload, &report).expect("installed");
    assert!(answered, "main.py answered over USB");
    assert_eq!(plan, Plan { firmware: true, model: true });
    {
        let k = k210.lock().unwrap();
        let image = isp::firmware_image(&payload.firmware);
        assert_eq!(&k.flash[..image.len()], &image[..]);
        assert_eq!(&k.flash[MODEL_AT as usize..MODEL_AT as usize + payload.model.len()], &payload.model[..]);
        assert_eq!(k.files["/flash/main.py"], payload.main);
        assert_eq!(k.files["/flash/boot.py"], BOOT_PY);
        assert_eq!(Stamp::parse(&String::from_utf8_lossy(&k.files[STAMP_FILE])), Some(stamp.clone()));
    }
    assert!(stamp.same_program(&payload.stamp("")) && stamp.firmware.contains("installed by Catalyst"));
    let seen = steps.lock().unwrap().clone();
    assert!(seen.iter().any(|(s, ..)| *s == Step::Firmware) && seen.iter().any(|(s, ..)| *s == Step::Model) && seen.last().is_some_and(|(s, d, _)| *s == Step::Check && *d == 1.0));
    // Asked again, it is up to date.
    let (_, found) = probe(&board, None);
    assert_eq!(need(&State { board: Some(board.clone()), found, job: Job::Idle }, Some(&payload.stamp(""))), Need::Current);

    // An older Catalyst's main.py on a camera with a full MaixPy and this
    // model: only main.py is written, no ISP at all.
    {
        let mut k = k210.lock().unwrap();
        let older = Stamp { main: "0a1b2c3d4e5f".into(), ..stamp.clone() };
        k.files.insert(STAMP_FILE.into(), older.json().into_bytes());
        k.log.clear();
    }
    let (answered, plan, _) = install_on(&board, Some("/dev/ttyUSB1".into()), &payload, &report).expect("updated");
    assert!(answered);
    assert_eq!(plan, Plan { firmware: false, model: false });
    assert!(!k210.lock().unwrap().log.iter().any(|l| l == "isp"), "no ISP for main.py alone");
    sim::plug_with(None);
}
