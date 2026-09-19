use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use maa_core::{Assistant, Callback};
use maa_types::{InstanceOptionKey, MessageKind};

const DYLIB: &str = "/Applications/MAA.app/Contents/Frameworks/libMaaCore.dylib";
const RESOURCE: &str = "/Applications/MAA.app/Contents/Resources";
const USER_DIR: &str = "/Users/diobrando/arkd/state/spike";
const PLAYTOOLS_ADDR: &str = "127.0.0.1:1717";
const SPIKE_OUT: &str = "/Users/diobrando/arkd/docs/spike";

struct Recorder {
    entries: Mutex<Vec<(MessageKind, Option<String>)>>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }
}

impl Callback for Recorder {
    fn on_message(&self, kind: MessageKind, msg: Option<&str>) {
        self.entries
            .lock()
            .unwrap()
            .push((kind, msg.map(String::from)));
    }
}

fn ensure_loaded() {
    if !Assistant::loaded() {
        Assistant::load(Path::new(DYLIB)).expect("load libMaaCore.dylib");
    }
}

fn playtools_reachable() -> bool {
    PLAYTOOLS_ADDR
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .map(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok())
        .unwrap_or(false)
}

struct PlayTools {
    stream: TcpStream,
}

impl PlayTools {
    fn connect() -> std::io::Result<Self> {
        let addr = PLAYTOOLS_ADDR.to_socket_addrs().unwrap().next().unwrap();
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        stream.set_read_timeout(Some(Duration::from_secs(15)))?;
        stream.set_nodelay(true)?;
        stream.write_all(b"MAA\0")?;
        let mut ok = [0u8; 4];
        stream.read_exact(&mut ok)?;
        if &ok != b"OKAY" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("handshake reply {ok:?}"),
            ));
        }
        Ok(Self { stream })
    }

    fn send(&mut self, payload: &[u8]) -> std::io::Result<()> {
        let len = u16::try_from(payload.len()).unwrap();
        self.stream.write_all(&len.to_be_bytes())?;
        self.stream.write_all(payload)?;
        self.stream.flush()
    }

    fn read_n(&mut self, n: usize) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; n];
        self.stream.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn version(&mut self) -> std::io::Result<u32> {
        self.send(b"VERN")?;
        let b = self.read_n(4)?;
        Ok(u32::from_be_bytes(b.try_into().unwrap()))
    }

    fn size(&mut self) -> std::io::Result<(u16, u16)> {
        self.send(b"SIZE")?;
        let b = self.read_n(4)?;
        Ok((
            u16::from_be_bytes(b[..2].try_into().unwrap()),
            u16::from_be_bytes(b[2..4].try_into().unwrap()),
        ))
    }

    fn scrn(&mut self) -> std::io::Result<Vec<u8>> {
        self.send(b"SCRN")?;
        let h = self.read_n(4)?;
        let n = u32::from_be_bytes(h.try_into().unwrap()) as usize;
        self.read_n(n)
    }

    fn bgr(&mut self) -> std::io::Result<(u32, u32, Vec<u8>)> {
        self.send(b"BGR\x01")?;
        let h = self.read_n(12)?;
        let w = u32::from_be_bytes(h[0..4].try_into().unwrap());
        let hh = u32::from_be_bytes(h[4..8].try_into().unwrap());
        let n = u32::from_be_bytes(h[8..12].try_into().unwrap()) as usize;
        let px = self.read_n(n)?;
        Ok((w, hh, px))
    }
}

fn main() {
    let step = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    let steps: Vec<&str> = if step == "all" {
        vec![
            "load",
            "resource",
            "callback",
            "two-instances",
            "connect",
            "playtools-concurrent",
            "hud-sample",
        ]
    } else {
        vec![step.as_str()]
    };

    for s in steps {
        match s {
            "load" => step_load(),
            "resource" => step_resource(),
            "callback" => step_callback(),
            "two-instances" => step_two_instances(),
            "connect" => step_connect(),
            "playtools-concurrent" => step_playtools_concurrent(),
            "hud-sample" => step_hud_sample(),
            other => println!("UNKNOWN step {other}"),
        }
    }
}

fn step_load() {
    let t = Instant::now();
    match Assistant::load(Path::new(DYLIB)) {
        Ok(()) => match Assistant::get_version() {
            Ok(v) => println!("PASS load version={v} load_ms={}", t.elapsed().as_millis()),
            Err(e) => println!("FAIL load get_version error={e}"),
        },
        Err(e) => println!("FAIL load error={e}"),
    }
}

fn step_resource() {
    ensure_loaded();
    let t = Instant::now();
    let r = Assistant::set_user_dir(USER_DIR).and_then(|()| Assistant::load_resource(RESOURCE));
    match r {
        Ok(()) => println!("PASS resource load_ms={}", t.elapsed().as_millis()),
        Err(e) => println!("FAIL resource error={e}"),
    }
}

fn step_callback() {
    ensure_loaded();
    let rec = Arc::new(Recorder::new());
    match Assistant::new_with_callback(rec.clone()) {
        Ok(asst) => {
            drop(asst);
            let n = rec.entries.lock().unwrap().len();
            println!("PASS callback created+dropped callbacks_seen={n}");
        }
        Err(e) => println!("FAIL callback error={e}"),
    }
}

fn step_two_instances() {
    ensure_loaded();
    let r1 = Arc::new(Recorder::new());
    let r2 = Arc::new(Recorder::new());
    let a = Assistant::new_with_callback(r1);
    let b = Assistant::new_with_callback(r2);
    match (a, b) {
        (Ok(a), Ok(b)) => {
            drop(a);
            drop(b);
            println!("PASS two-instances both alive then dropped");
        }
        (a, b) => {
            drop(a);
            drop(b);
            println!("FAIL two-instances one creation failed");
        }
    }
}

fn step_connect() {
    ensure_loaded();
    if !playtools_reachable() {
        println!("SKIP connect game not running");
        return;
    }
    connect_and_capture("General", "frame-general.png");
    connect_and_capture("MacBGR", "frame-macbgr.png");
}

fn connect_and_capture(config: &str, filename: &str) {
    let rec = Arc::new(Recorder::new());
    let asst = match Assistant::new_with_callback(rec.clone()) {
        Ok(a) => a,
        Err(e) => {
            println!("FAIL connect[{config}] create error={e}");
            return;
        }
    };
    if let Err(e) = asst.set_instance_option(InstanceOptionKey::TouchMode, "MacPlayTools") {
        println!("FAIL connect[{config}] set_instance_option error={e}");
        return;
    }
    let t = Instant::now();
    match asst.async_connect("", PLAYTOOLS_ADDR, config, true) {
        Ok(_) => {
            let conn_msgs: Vec<String> = rec
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter(|(k, _)| matches!(k, MessageKind::ConnectionInfo))
                .filter_map(|(_, m)| m.clone())
                .collect();
            println!(
                "PASS connect[{config}] connected={} ms={} conn_msgs={}",
                asst.connected(),
                t.elapsed().as_millis(),
                conn_msgs.join(" | ")
            );
            let t2 = Instant::now();
            match asst.async_screencap(true).and_then(|_| asst.get_image()) {
                Ok(Some(png)) => {
                    let ms = t2.elapsed().as_millis();
                    match image::load_from_memory(&png) {
                        Ok(img) => {
                            let out = format!("{SPIKE_OUT}/{filename}");
                            std::fs::write(&out, &png).ok();
                            println!(
                                "PASS screencap[{config}] {}x{} bytes={} ms={ms} saved={out}",
                                img.width(),
                                img.height(),
                                png.len()
                            );
                        }
                        Err(e) => println!("FAIL screencap[{config}] decode error={e}"),
                    }
                }
                Ok(None) => println!("FAIL screencap[{config}] no image cached"),
                Err(e) => println!("FAIL screencap[{config}] error={e}"),
            }
        }
        Err(e) => println!("FAIL connect[{config}] error={e}"),
    }
}

fn step_playtools_concurrent() {
    ensure_loaded();
    if !playtools_reachable() {
        println!("SKIP playtools-concurrent game not running");
        return;
    }

    let rec = Arc::new(Recorder::new());
    let asst = match Assistant::new_with_callback(rec) {
        Ok(a) => a,
        Err(e) => {
            println!("FAIL playtools-concurrent create error={e}");
            return;
        }
    };
    if let Err(e) = asst.set_instance_option(InstanceOptionKey::TouchMode, "MacPlayTools") {
        println!("FAIL playtools-concurrent set_instance_option error={e}");
        return;
    }
    if let Err(e) = asst.async_connect("", PLAYTOOLS_ADDR, "General", true) {
        println!("FAIL playtools-concurrent connect error={e}");
        return;
    }

    let mut pt = match PlayTools::connect() {
        Ok(p) => p,
        Err(e) => {
            println!("FAIL playtools-concurrent second connection rejected error={e}");
            return;
        }
    };
    println!("PASS playtools-concurrent second connection accepted");

    let ver = match pt.version() {
        Ok(v) => {
            println!("PASS VERN version={v}");
            v
        }
        Err(e) => {
            println!("FAIL VERN error={e}");
            0
        }
    };
    match pt.size() {
        Ok((w, h)) => println!("PASS SIZE {w}x{h}"),
        Err(e) => println!("FAIL SIZE error={e}"),
    }

    let t = Instant::now();
    let cap = if ver >= 3 {
        pt.bgr().map(|(w, h, px)| (w, h, 3usize, px))
    } else {
        pt.scrn().map(|px| (0, 0, 4usize, px))
    };
    match cap {
        Ok((w, h, ch, px)) => {
            println!(
                "PASS raw capture {}x{} channels={ch} bytes={} ms={}",
                w,
                h,
                px.len(),
                t.elapsed().as_millis()
            );
            let out = format!("{SPIKE_OUT}/frame-playtools.raw");
            std::fs::write(&out, &px).ok();
        }
        Err(e) => println!("FAIL raw capture error={e}"),
    }

    let t2 = Instant::now();
    match asst.async_screencap(true).and_then(|_| asst.get_image()) {
        Ok(Some(png)) => println!(
            "PASS MaaCore screencap while raw socket open bytes={} ms={}",
            png.len(),
            t2.elapsed().as_millis()
        ),
        other => println!("FAIL MaaCore screencap while raw socket open result={other:?}"),
    }
}

fn step_hud_sample() {
    ensure_loaded();
    if !playtools_reachable() {
        println!("SKIP needs formation screen");
        return;
    }
    let rec = Arc::new(Recorder::new());
    let asst = match Assistant::new_with_callback(rec) {
        Ok(a) => a,
        Err(e) => {
            println!("FAIL hud-sample create error={e}");
            return;
        }
    };
    if asst
        .set_instance_option(InstanceOptionKey::TouchMode, "MacPlayTools")
        .and_then(|()| asst.async_connect("", PLAYTOOLS_ADDR, "General", true))
        .is_err()
    {
        println!("SKIP needs formation screen");
        return;
    }
    match asst.async_screencap(true).and_then(|_| asst.get_image()) {
        Ok(Some(png)) => {
            std::fs::write(format!("{SPIKE_OUT}/formation.png"), &png).ok();
            println!("PASS hud-sample saved {SPIKE_OUT}/formation.png");
        }
        _ => println!("FAIL hud-sample no image"),
    }
}
