use std::time::Duration;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::error::{Error, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum TouchPhase {
    Began = 0,
    Moved = 1,
    Ended = 3,
}

#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgr: Vec<u8>,
}

impl Frame {
    pub fn to_png(&self) -> Result<Vec<u8>> {
        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);
        for px in self.bgr.as_chunks::<3>().0 {
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
        let img = image::RgbaImage::from_raw(self.width, self.height, rgba).ok_or_else(|| {
            Error::Image(format!(
                "frame dimensions {}x{} do not match {} bytes",
                self.width,
                self.height,
                self.bgr.len()
            ))
        })?;
        let mut out = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut out);
        image::ImageEncoder::write_image(
            encoder,
            img.as_raw(),
            self.width,
            self.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| Error::Image(format!("PNG encode failed: {e}")))?;
        Ok(out)
    }
}

#[derive(Debug)]
pub struct PlayToolsClient {
    stream: TcpStream,
    timeout: Duration,
}

impl PlayToolsClient {
    pub async fn connect(addr: &str, timeout: Duration) -> Result<Self> {
        let timeout = if timeout.is_zero() {
            DEFAULT_TIMEOUT
        } else {
            timeout
        };
        let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| Error::PlayTools(format!("connect to {addr} timed out")))?
            .map_err(|e| Error::PlayTools(format!("connect to {addr} failed: {e}")))?;
        stream.set_nodelay(true).ok();
        let mut client = Self { stream, timeout };
        tokio::time::timeout(timeout, client.handshake())
            .await
            .map_err(|_| Error::PlayTools("handshake timed out".to_string()))??;
        Ok(client)
    }

    async fn handshake(&mut self) -> Result<()> {
        self.stream.write_all(b"MAA\0").await?;
        let mut ok = [0u8; 4];
        self.stream.read_exact(&mut ok).await?;
        if &ok != b"OKAY" {
            return Err(Error::PlayTools(format!("handshake refused: {ok:?}")));
        }
        Ok(())
    }

    async fn request(&mut self, payload: &[u8]) -> Result<()> {
        let len = u16::try_from(payload.len())
            .map_err(|_| Error::PlayTools("request too large".to_string()))?;
        tokio::time::timeout(self.timeout, async {
            self.stream.write_all(&len.to_be_bytes()).await?;
            self.stream.write_all(payload).await?;
            self.stream.flush().await
        })
        .await
        .map_err(|_| Error::PlayTools("request timed out".to_string()))??;
        Ok(())
    }

    async fn read_n(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; n];
        tokio::time::timeout(self.timeout, self.stream.read_exact(&mut buf))
            .await
            .map_err(|_| Error::PlayTools("read timed out".to_string()))??;
        Ok(buf)
    }

    pub async fn version(&mut self) -> Result<u32> {
        self.request(b"VERN").await?;
        let b = self.read_n(4).await?;
        Ok(u32::from_be_bytes(b.try_into().unwrap()))
    }

    pub async fn size(&mut self) -> Result<(u16, u16)> {
        self.request(b"SIZE").await?;
        let b = self.read_n(4).await?;
        Ok((
            u16::from_be_bytes(b[..2].try_into().unwrap()),
            u16::from_be_bytes(b[2..4].try_into().unwrap()),
        ))
    }

    pub async fn bundle_id(&mut self) -> Result<String> {
        self.request(b"BNDL").await?;
        let b = self.read_n(2).await?;
        let n = u16::from_be_bytes(b.try_into().unwrap()) as usize;
        let s = self.read_n(n).await?;
        String::from_utf8(s).map_err(|e| Error::PlayTools(format!("BNDL reply: {e}")))
    }

    pub async fn capture_rgba(&mut self) -> Result<Frame> {
        self.request(b"SCRN").await?;
        let h = self.read_n(4).await?;
        let n = u32::from_be_bytes(h.try_into().unwrap()) as usize;
        let rgba = self.read_n(n).await?;
        let (w, hgt) = self.size().await?;
        let mut bgr = Vec::with_capacity((w as usize) * (hgt as usize) * 3);
        for px in rgba.as_chunks::<4>().0 {
            bgr.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        Ok(Frame {
            width: w as u32,
            height: hgt as u32,
            bgr,
        })
    }

    pub async fn capture_bgr(&mut self) -> Result<Frame> {
        self.request(b"BGR\x01").await?;
        let h = self.read_n(12).await?;
        let w = u32::from_be_bytes(h[0..4].try_into().unwrap());
        let hgt = u32::from_be_bytes(h[4..8].try_into().unwrap());
        let n = u32::from_be_bytes(h[8..12].try_into().unwrap()) as usize;
        let bgr = self.read_n(n).await?;
        Ok(Frame {
            width: w,
            height: hgt,
            bgr,
        })
    }

    pub async fn capture(&mut self) -> Result<Frame> {
        if self.version().await? >= 3 {
            self.capture_bgr().await
        } else {
            self.capture_rgba().await
        }
    }

    pub async fn touch(&mut self, phase: TouchPhase, x: u16, y: u16, contact: u8) -> Result<()> {
        let mut payload = Vec::with_capacity(10);
        payload.extend_from_slice(b"TUCH");
        payload.push(phase as u8);
        payload.extend_from_slice(&x.to_be_bytes());
        payload.extend_from_slice(&y.to_be_bytes());
        payload.push(contact);
        self.request(&payload).await
    }

    pub async fn tap(&mut self, x: u16, y: u16, hold: Duration) -> Result<()> {
        self.touch(TouchPhase::Began, x, y, 0).await?;
        tokio::time::sleep(hold).await;
        self.touch(TouchPhase::Ended, x, y, 0).await
    }

    pub async fn drag(
        &mut self,
        points: &[(u16, u16)],
        hold: Duration,
        step: Duration,
    ) -> Result<()> {
        let Some(&(x0, y0)) = points.first() else {
            return Err(Error::PlayTools(
                "drag needs at least one point".to_string(),
            ));
        };
        self.touch(TouchPhase::Began, x0, y0, 0).await?;
        for &(x, y) in &points[1..] {
            tokio::time::sleep(step).await;
            self.touch(TouchPhase::Moved, x, y, 0).await?;
        }
        tokio::time::sleep(hold).await;
        let &(xe, ye) = points.last().unwrap();
        self.touch(TouchPhase::Ended, xe, ye, 0).await
    }
}

#[cfg(any(test, feature = "fake"))]
pub mod fake {
    use std::{
        net::SocketAddr,
        sync::{Arc, Mutex},
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        task::JoinHandle,
    };

    use super::Frame;

    #[derive(Clone, Debug)]
    pub struct TouchEvent {
        pub phase: u8,
        pub x: u16,
        pub y: u16,
        pub contact: u8,
    }

    pub struct FakePlayToolsServer {
        pub addr: SocketAddr,
        pub touches: Arc<Mutex<Vec<TouchEvent>>>,
        pub handle: JoinHandle<()>,
    }

    impl FakePlayToolsServer {
        pub async fn spawn(frame: Frame, version: u32) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let touches: Arc<Mutex<Vec<TouchEvent>>> = Arc::new(Mutex::new(Vec::new()));
            let t2 = touches.clone();
            let handle = tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let t = t2.clone();
                    let frame_bytes = frame.bgr.clone();
                    let (w, h) = (frame.width, frame.height);
                    tokio::spawn(async move {
                        let _ = serve_conn(stream, t, frame_bytes, w, h, version).await;
                    });
                }
            });
            Self {
                addr,
                touches,
                handle,
            }
        }
    }

    async fn serve_conn(
        mut stream: TcpStream,
        touches: Arc<Mutex<Vec<TouchEvent>>>,
        frame_bgr: Vec<u8>,
        w: u32,
        h: u32,
        version: u32,
    ) -> std::io::Result<()> {
        let mut hello = [0u8; 4];
        stream.read_exact(&mut hello).await?;
        if &hello != b"MAA\0" {
            return Ok(());
        }
        stream.write_all(b"OKAY").await?;
        loop {
            let mut len = [0u8; 2];
            if stream.read_exact(&mut len).await.is_err() {
                return Ok(());
            }
            let n = u16::from_be_bytes(len) as usize;
            let mut payload = vec![0u8; n];
            if stream.read_exact(&mut payload).await.is_err() {
                return Ok(());
            }
            match payload.as_slice() {
                b"VERN" => {
                    stream.write_all(&version.to_be_bytes()).await?;
                }
                b"SIZE" => {
                    let mut out = Vec::with_capacity(4);
                    out.extend_from_slice(&(w as u16).to_be_bytes());
                    out.extend_from_slice(&(h as u16).to_be_bytes());
                    stream.write_all(&out).await?;
                }
                b"BNDL" => {
                    let name = b"com.hypergryph.arknights";
                    stream.write_all(&(name.len() as u16).to_be_bytes()).await?;
                    stream.write_all(name).await?;
                }
                b"SCRN" => {
                    let mut rgba = Vec::with_capacity(frame_bgr.len() / 3 * 4);
                    for px in frame_bgr.as_chunks::<3>().0 {
                        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                    }
                    stream.write_all(&(rgba.len() as u32).to_be_bytes()).await?;
                    stream.write_all(&rgba).await?;
                }
                p if p == b"BGR\x01" => {
                    let mut hdr = Vec::with_capacity(12);
                    hdr.extend_from_slice(&w.to_be_bytes());
                    hdr.extend_from_slice(&h.to_be_bytes());
                    hdr.extend_from_slice(&(frame_bgr.len() as u32).to_be_bytes());
                    stream.write_all(&hdr).await?;
                    stream.write_all(&frame_bgr).await?;
                }
                p if p.starts_with(b"TUCH") && p.len() >= 10 => {
                    touches.lock().unwrap().push(TouchEvent {
                        phase: p[4],
                        x: u16::from_be_bytes(p[5..7].try_into().unwrap()),
                        y: u16::from_be_bytes(p[7..9].try_into().unwrap()),
                        contact: p[9],
                    });
                }
                _ => {}
            }
        }
    }
}
