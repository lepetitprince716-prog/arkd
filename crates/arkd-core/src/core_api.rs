use std::{
    ffi::{CStr, CString, c_char, c_void},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use maa_ffi_types::AsstSize;
use maa_sys::binding::{self, AsstHandle};
use maa_types::InstanceOptionKey;

use crate::error::{Error, Result};

pub type CallbackFn = Arc<dyn Fn(i32, &str) + Send + Sync>;

pub trait MaaCoreApi: Send + Sync {
    fn version(&self) -> String;
    fn set_instance_option(&self, key: InstanceOptionKey, value: &str) -> Result<()>;
    fn connect(&self, adb_path: &str, address: &str, config: &str) -> Result<()>;
    fn connected(&self) -> bool;
    fn append_task(&self, task_type: &str, params_json: &str) -> Result<i32>;
    fn set_task_params(&self, task_id: i32, params_json: &str) -> Result<()>;
    fn start(&self) -> Result<()>;
    fn stop(&self) -> Result<()>;
    fn running(&self) -> bool;
    fn back_to_home(&self) -> Result<()>;
    fn screencap(&self) -> Result<()>;
    fn image_png(&self) -> Result<Option<Vec<u8>>>;
    fn image_bgr(&self) -> Result<Option<Vec<u8>>>;
    fn click(&self, x: i32, y: i32) -> Result<()>;
    fn uuid(&self) -> Option<String>;
}

pub trait CoreFactory: Send + Sync {
    fn create(&self, callback: CallbackFn) -> Result<Box<dyn MaaCoreApi>>;
}

pub struct RuntimeInfo {
    pub version: String,
}

pub struct Runtime;

static RUNTIME_INFO: OnceLock<RuntimeInfo> = OnceLock::new();

fn core_library_path(core_dir: &Path) -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let name = "libMaaCore.dylib";
    #[cfg(target_os = "windows")]
    let name = "MaaCore.dll";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err(Error::CoreNotFound(format!(
        "arkd does not support this platform; MaaCore is only loaded on macOS and Windows."
    )));
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let path = core_dir.join(name);
        if !path.is_file() {
            return Err(Error::CoreNotFound(format!(
                "No MaaCore library at {}. Point maa.core_dir at an installed MAA directory.",
                path.display()
            )));
        }
        Ok(path)
    }
}

impl Runtime {
    pub fn init(
        core_dir: &Path,
        resource_dir: &Path,
        user_dir: &Path,
        incremental: &[PathBuf],
    ) -> Result<&'static RuntimeInfo> {
        if let Some(info) = RUNTIME_INFO.get() {
            return Ok(info);
        }
        let library = core_library_path(core_dir)?;
        maa_core::Assistant::load(&library).map_err(|e| {
            Error::CoreNotFound(format!("Failed to load {}: {e}", library.display()))
        })?;
        std::fs::create_dir_all(user_dir)?;
        maa_core::Assistant::set_user_dir(user_dir.to_string_lossy().as_ref()).map_err(|e| {
            Error::CoreLoad(format!(
                "MaaCore refused the user directory {}: {e}",
                user_dir.display()
            ))
        })?;
        if !resource_dir.join("resource").is_dir() {
            return Err(Error::CoreLoad(format!(
                "No 'resource' directory under {}. MaaCore needs MAA's resource tree next to the library; point maa.resource_dir at an installed MAA directory.",
                resource_dir.display()
            )));
        }
        maa_core::Assistant::load_resource(resource_dir.to_string_lossy().as_ref()).map_err(|e| {
            Error::CoreLoad(format!(
                "MaaCore rejected the resource directory {}. The tree is present but could not be parsed -- it is usually a partial download or a resource version that does not match this MaaCore build. ({e})",
                resource_dir.display()
            ))
        })?;
        for extra in incremental {
            maa_core::Assistant::load_resource(extra.to_string_lossy().as_ref()).map_err(|e| {
                Error::CoreLoad(format!(
                    "MaaCore rejected the incremental resource directory {}. Incremental resources layer on top of the main one and must be a matching overlay (e.g. resource/global/YoStarEN). ({e})",
                    extra.display()
                ))
            })?;
        }
        let version = maa_core::Assistant::get_version()
            .map_err(|e| Error::CoreLoad(format!("MaaCore loaded but get_version failed: {e}")))?;
        Ok(RUNTIME_INFO.get_or_init(|| RuntimeInfo { version }))
    }
}

fn cstr(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::CoreLoad(format!("string contains a NUL byte: {s:?}")))
}

struct CallbackBox(CallbackFn);

// Safety: `userdata` is the address of a live `CallbackBox` owned by `RealCore`;
// the box is only freed after `AsstDestroy`, which stops MaaCore's threads from
// invoking the trampoline again. The message pointer is a NUL-terminated string
// valid for the duration of the call. A panic must never cross the FFI
// boundary, so the body runs under catch_unwind.
unsafe extern "C" fn trampoline(msg_id: i32, msg: *const c_char, userdata: *mut c_void) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let cb = unsafe { &*(userdata as *const CallbackBox) };
        let text = if msg.is_null() {
            "{}".to_string()
        } else {
            unsafe { CStr::from_ptr(msg) }
                .to_str()
                .unwrap_or("{}")
                .to_string()
        };
        (cb.0)(msg_id, &text);
    }));
}

pub struct RealCore {
    handle: AsstHandle,
    _callback: Box<CallbackBox>,
}

// Safety: MaaCore serialises access to an assistant handle internally; the
// handle is only ever passed back to MaaCore's own API, never dereferenced.
unsafe impl Send for RealCore {}
// Safety: same argument; concurrent calls go through MaaCore's own locking.
unsafe impl Sync for RealCore {}

impl Drop for RealCore {
    fn drop(&mut self) {
        // Safety: handle was returned by AsstCreateEx and has not been destroyed.
        unsafe { binding::AsstDestroy(self.handle) };
    }
}

fn bool_result(ok: u8, what: &str) -> Result<()> {
    if ok != 0 {
        Ok(())
    } else {
        Err(Error::CoreLoad(format!(
            "{what} returned false; check the MaaCore log for details"
        )))
    }
}

const NULL_SIZE: AsstSize = AsstSize::MAX;

fn read_growable(
    mut call: impl FnMut(*mut c_void, AsstSize) -> AsstSize,
) -> Result<Option<Vec<u8>>> {
    let mut buf_size: usize = 1024 * 1024 * 4;
    const MAX_SIZE: usize = 1024 * 1024 * 64;
    let mut buf = Vec::with_capacity(buf_size);
    loop {
        let n = call(buf.as_mut_ptr() as *mut c_void, buf_size as AsstSize);
        if n == NULL_SIZE {
            if buf_size >= MAX_SIZE {
                return Err(Error::CoreLoad(format!(
                    "MaaCore returned a frame larger than {MAX_SIZE} bytes"
                )));
            }
            buf_size *= 2;
            buf.reserve(buf_size);
            continue;
        }
        if n == 0 {
            return Ok(None);
        }
        // Safety: MaaCore reported writing exactly n bytes into buf.
        unsafe { buf.set_len(n as usize) };
        return Ok(Some(buf));
    }
}

impl MaaCoreApi for RealCore {
    fn version(&self) -> String {
        // Safety: the pointer lives in the loaded library's data segment.
        unsafe { CStr::from_ptr(binding::AsstGetVersion()) }
            .to_string_lossy()
            .into_owned()
    }

    fn set_instance_option(&self, key: InstanceOptionKey, value: &str) -> Result<()> {
        let v = cstr(value)?;
        // Safety: handle is live; v is NUL-terminated.
        let ok = unsafe { binding::AsstSetInstanceOption(self.handle, key as i32, v.as_ptr()) };
        bool_result(ok, "AsstSetInstanceOption")
    }

    fn connect(&self, adb_path: &str, address: &str, config: &str) -> Result<()> {
        let a = cstr(adb_path)?;
        let b = cstr(address)?;
        let c = cstr(config)?;
        // Safety: handle is live; all pointers are NUL-terminated strings.
        let id = unsafe {
            binding::AsstAsyncConnect(self.handle, a.as_ptr(), b.as_ptr(), c.as_ptr(), 1)
        };
        if id <= 0 {
            return Err(Error::DeviceConnection(
                "AsstAsyncConnect returned a failure id".to_string(),
            ));
        }
        Ok(())
    }

    fn connected(&self) -> bool {
        // Safety: handle is live.
        unsafe { binding::AsstConnected(self.handle) != 0 }
    }

    fn append_task(&self, task_type: &str, params_json: &str) -> Result<i32> {
        let t = cstr(task_type)?;
        let p = cstr(params_json)?;
        // Safety: handle is live; pointers are NUL-terminated.
        Ok(unsafe { binding::AsstAppendTask(self.handle, t.as_ptr(), p.as_ptr()) })
    }

    fn set_task_params(&self, task_id: i32, params_json: &str) -> Result<()> {
        let p = cstr(params_json)?;
        // Safety: handle is live; p is NUL-terminated.
        let ok = unsafe { binding::AsstSetTaskParams(self.handle, task_id, p.as_ptr()) };
        bool_result(ok, "AsstSetTaskParams")
    }

    fn start(&self) -> Result<()> {
        // Safety: handle is live.
        bool_result(unsafe { binding::AsstStart(self.handle) }, "AsstStart")
    }

    fn stop(&self) -> Result<()> {
        // Safety: handle is live.
        bool_result(unsafe { binding::AsstStop(self.handle) }, "AsstStop")
    }

    fn running(&self) -> bool {
        // Safety: handle is live.
        unsafe { binding::AsstRunning(self.handle) != 0 }
    }

    fn back_to_home(&self) -> Result<()> {
        // Safety: handle is live.
        bool_result(
            unsafe { binding::AsstBackToHome(self.handle) },
            "AsstBackToHome",
        )
    }

    fn screencap(&self) -> Result<()> {
        // Safety: handle is live.
        let id = unsafe { binding::AsstAsyncScreencap(self.handle, 1) };
        if id <= 0 {
            return Err(Error::CoreLoad(
                "AsstAsyncScreencap returned a failure id".to_string(),
            ));
        }
        Ok(())
    }

    fn image_png(&self) -> Result<Option<Vec<u8>>> {
        let handle = self.handle;
        read_growable(|buf, size| unsafe { binding::AsstGetImage(handle, buf, size) })
    }

    fn image_bgr(&self) -> Result<Option<Vec<u8>>> {
        let handle = self.handle;
        read_growable(|buf, size| unsafe { binding::AsstGetImageBgr(handle, buf, size) })
    }

    fn click(&self, x: i32, y: i32) -> Result<()> {
        // Safety: handle is live.
        let id = unsafe { binding::AsstAsyncClick(self.handle, x, y, 1) };
        if id <= 0 {
            return Err(Error::CoreLoad(
                "AsstAsyncClick returned a failure id".to_string(),
            ));
        }
        Ok(())
    }

    fn uuid(&self) -> Option<String> {
        let handle = self.handle;
        match read_growable(|buf, size| unsafe {
            binding::AsstGetUUID(handle, buf as *mut c_char, size)
        }) {
            Ok(Some(bytes)) => String::from_utf8(bytes).ok(),
            _ => None,
        }
    }
}

pub struct RealCoreFactory;

impl CoreFactory for RealCoreFactory {
    fn create(&self, callback: CallbackFn) -> Result<Box<dyn MaaCoreApi>> {
        if !binding::loaded() {
            return Err(Error::CoreNotFound(
                "MaaCore is not loaded; call Runtime::init first".to_string(),
            ));
        }
        let callback = Box::new(CallbackBox(callback));
        let raw = &raw const *callback as *mut c_void;
        // Safety: trampoline is a valid extern "C" fn; userdata points at the
        // boxed callback, kept alive inside the returned RealCore.
        let handle = unsafe { binding::AsstCreateEx(Some(trampoline), raw) };
        if handle.is_null() {
            return Err(Error::CoreLoad(
                "AsstCreateEx returned a null handle; is the runtime loaded?".to_string(),
            ));
        }
        Ok(Box::new(RealCore {
            handle,
            _callback: callback,
        }))
    }
}

#[cfg(any(test, feature = "fake"))]
pub mod fake {
    use std::sync::{Arc, Mutex};

    use super::*;

    pub struct FakeCore {
        version: String,
        callback: Mutex<Option<CallbackFn>>,
        pub screencap_calls: Mutex<u32>,
        pub clicks: Mutex<Vec<(i32, i32)>>,
        pub connect_calls: Mutex<Vec<(String, String, String)>>,
        pub instance_options: Mutex<Vec<(i32, String)>>,
        pub back_to_home_calls: Mutex<u32>,
        pub tasks: Mutex<Vec<(i32, String, serde_json::Value)>>,
        connected: Mutex<bool>,
        running: Mutex<bool>,
        next_task_id: Mutex<i32>,
        connect_ok: Mutex<bool>,
        append_ok: Mutex<bool>,
        start_ok: Mutex<bool>,
        image_png: Mutex<Option<Vec<u8>>>,
        fresh_image_png: Mutex<Option<Vec<u8>>>,
        image_bgr: Mutex<Option<Vec<u8>>>,
        auto_finish: Mutex<Option<std::time::Duration>>,
        running_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    }

    impl Default for FakeCore {
        fn default() -> Self {
            Self::new()
        }
    }

    impl FakeCore {
        pub fn new() -> Self {
            Self {
                version: "v5.99.0-fake".to_string(),
                callback: Mutex::new(None),
                screencap_calls: Mutex::new(0),
                clicks: Mutex::new(Vec::new()),
                connect_calls: Mutex::new(Vec::new()),
                instance_options: Mutex::new(Vec::new()),
                back_to_home_calls: Mutex::new(0),
                tasks: Mutex::new(Vec::new()),
                connected: Mutex::new(false),
                running: Mutex::new(false),
                next_task_id: Mutex::new(1),
                connect_ok: Mutex::new(true),
                append_ok: Mutex::new(true),
                start_ok: Mutex::new(true),
                image_png: Mutex::new(Some(b"\x89PNG\r\n\x1a\nfake".to_vec())),
                fresh_image_png: Mutex::new(None),
                image_bgr: Mutex::new(None),
                auto_finish: Mutex::new(None),
                running_hook: Mutex::new(None),
            }
        }

        pub fn emit(&self, message_id: i32, payload: serde_json::Value) {
            self.emit_raw(message_id, &payload.to_string());
        }

        pub fn emit_raw(&self, message_id: i32, details_json: &str) {
            let cb = self.callback.lock().unwrap().clone();
            if let Some(cb) = cb {
                cb(message_id, details_json);
            }
        }

        pub fn set_connected(&self, v: bool) {
            *self.connected.lock().unwrap() = v;
        }

        pub fn set_running(&self, v: bool) {
            *self.running.lock().unwrap() = v;
        }

        pub fn set_connect_ok(&self, v: bool) {
            *self.connect_ok.lock().unwrap() = v;
        }

        pub fn refuse_append(&self, v: bool) {
            *self.append_ok.lock().unwrap() = !v;
        }

        pub fn refuse_start(&self, v: bool) {
            *self.start_ok.lock().unwrap() = !v;
        }

        pub fn set_image_png(&self, bytes: Option<Vec<u8>>) {
            *self.image_png.lock().unwrap() = bytes;
        }

        pub fn set_fresh_image_png(&self, bytes: Option<Vec<u8>>) {
            *self.fresh_image_png.lock().unwrap() = bytes;
        }

        pub fn set_image_bgr(&self, bytes: Option<Vec<u8>>) {
            *self.image_bgr.lock().unwrap() = bytes;
        }

        pub fn set_auto_finish(&self, after: std::time::Duration) {
            *self.auto_finish.lock().unwrap() = Some(after);
        }

        pub fn set_running_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
            *self.running_hook.lock().unwrap() = Some(hook);
        }

        pub fn finish_all(self: &Arc<Self>) {
            let tasks: Vec<(i32, String)> = self
                .tasks
                .lock()
                .unwrap()
                .iter()
                .map(|(id, t, _)| (*id, t.clone()))
                .collect();
            let finished: Vec<i32> = tasks.iter().map(|(id, _)| *id).collect();
            for (id, task_type) in &tasks {
                self.emit(
                    10002,
                    serde_json::json!({"taskchain": task_type, "taskid": id, "uuid": "fake-uuid"}),
                );
            }
            *self.running.lock().unwrap() = false;
            self.emit(
                3,
                serde_json::json!({"taskchain": "", "uuid": "fake-uuid", "finished_tasks": finished}),
            );
        }
    }

    impl MaaCoreApi for FakeCore {
        fn version(&self) -> String {
            self.version.clone()
        }

        fn set_instance_option(&self, key: InstanceOptionKey, value: &str) -> Result<()> {
            self.instance_options
                .lock()
                .unwrap()
                .push((key as i32, value.to_string()));
            Ok(())
        }

        fn connect(&self, adb_path: &str, address: &str, config: &str) -> Result<()> {
            self.connect_calls.lock().unwrap().push((
                adb_path.to_string(),
                address.to_string(),
                config.to_string(),
            ));
            if !*self.connect_ok.lock().unwrap() {
                self.emit(
                    2,
                    serde_json::json!({"what": "ConnectFailed", "why": "device offline", "uuid": ""}),
                );
                return Err(Error::DeviceConnection("connect refused".to_string()));
            }
            *self.connected.lock().unwrap() = true;
            self.emit(
                2,
                serde_json::json!({"what": "Connected", "uuid": "fake-uuid", "details": {"address": address}}),
            );
            Ok(())
        }

        fn connected(&self) -> bool {
            *self.connected.lock().unwrap()
        }

        fn append_task(&self, task_type: &str, params_json: &str) -> Result<i32> {
            if !*self.append_ok.lock().unwrap() {
                return Ok(0);
            }
            let mut next = self.next_task_id.lock().unwrap();
            let id = *next;
            *next += 1;
            let params: serde_json::Value =
                serde_json::from_str(params_json).unwrap_or(serde_json::Value::Null);
            self.tasks
                .lock()
                .unwrap()
                .push((id, task_type.to_string(), params));
            Ok(id)
        }

        fn set_task_params(&self, task_id: i32, params_json: &str) -> Result<()> {
            let params: serde_json::Value =
                serde_json::from_str(params_json).unwrap_or(serde_json::Value::Null);
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(entry) = tasks.iter_mut().find(|(id, _, _)| *id == task_id) {
                entry.2 = params;
                Ok(())
            } else {
                Err(Error::Refused(format!("no task {task_id}")))
            }
        }

        fn start(&self) -> Result<()> {
            if !*self.connected.lock().unwrap() || !*self.start_ok.lock().unwrap() {
                return Err(Error::CoreLoad("AsstStart returned false".to_string()));
            }
            *self.running.lock().unwrap() = true;
            let tasks: Vec<(i32, String)> = self
                .tasks
                .lock()
                .unwrap()
                .iter()
                .map(|(id, t, _)| (*id, t.clone()))
                .collect();
            for (id, task_type) in &tasks {
                self.emit(
                    10001,
                    serde_json::json!({"taskchain": task_type, "taskid": id, "uuid": "fake-uuid"}),
                );
            }
            Ok(())
        }

        fn stop(&self) -> Result<()> {
            *self.running.lock().unwrap() = false;
            self.tasks.lock().unwrap().clear();
            Ok(())
        }

        fn running(&self) -> bool {
            let hook = self.running_hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook();
            }
            *self.running.lock().unwrap()
        }

        fn back_to_home(&self) -> Result<()> {
            *self.back_to_home_calls.lock().unwrap() += 1;
            if *self.connected.lock().unwrap() {
                Ok(())
            } else {
                Err(Error::CoreLoad("not connected".to_string()))
            }
        }

        fn screencap(&self) -> Result<()> {
            *self.screencap_calls.lock().unwrap() += 1;
            if let Some(fresh) = self.fresh_image_png.lock().unwrap().clone() {
                *self.image_png.lock().unwrap() = Some(fresh);
            }
            if *self.connected.lock().unwrap() {
                Ok(())
            } else {
                Err(Error::CoreLoad("not connected".to_string()))
            }
        }

        fn image_png(&self) -> Result<Option<Vec<u8>>> {
            if *self.connected.lock().unwrap() {
                Ok(self.image_png.lock().unwrap().clone())
            } else {
                Ok(None)
            }
        }

        fn image_bgr(&self) -> Result<Option<Vec<u8>>> {
            if *self.connected.lock().unwrap() {
                Ok(self.image_bgr.lock().unwrap().clone())
            } else {
                Ok(None)
            }
        }

        fn click(&self, x: i32, y: i32) -> Result<()> {
            self.clicks.lock().unwrap().push((x, y));
            if *self.connected.lock().unwrap() {
                Ok(())
            } else {
                Err(Error::CoreLoad("not connected".to_string()))
            }
        }

        fn uuid(&self) -> Option<String> {
            if *self.connected.lock().unwrap() {
                Some("fake-uuid".to_string())
            } else {
                None
            }
        }
    }

    pub struct FakeCoreFactory {
        pub core: Arc<FakeCore>,
    }

    impl Default for FakeCoreFactory {
        fn default() -> Self {
            Self::new()
        }
    }

    impl FakeCoreFactory {
        pub fn new() -> Self {
            Self {
                core: Arc::new(FakeCore::new()),
            }
        }
    }

    impl CoreFactory for FakeCoreFactory {
        fn create(&self, callback: CallbackFn) -> Result<Box<dyn MaaCoreApi>> {
            *self.core.callback.lock().unwrap() = Some(callback);
            Ok(Box::new(FakeCoreHandle {
                core: self.core.clone(),
            }))
        }
    }

    struct FakeCoreHandle {
        core: Arc<FakeCore>,
    }

    impl MaaCoreApi for FakeCoreHandle {
        fn version(&self) -> String {
            self.core.version()
        }
        fn set_instance_option(&self, key: InstanceOptionKey, value: &str) -> Result<()> {
            self.core.set_instance_option(key, value)
        }
        fn connect(&self, adb_path: &str, address: &str, config: &str) -> Result<()> {
            self.core.connect(adb_path, address, config)
        }
        fn connected(&self) -> bool {
            self.core.connected()
        }
        fn append_task(&self, task_type: &str, params_json: &str) -> Result<i32> {
            self.core.append_task(task_type, params_json)
        }
        fn set_task_params(&self, task_id: i32, params_json: &str) -> Result<()> {
            self.core.set_task_params(task_id, params_json)
        }
        fn start(&self) -> Result<()> {
            self.core.start()?;
            if let Some(d) = *self.core.auto_finish.lock().unwrap() {
                let core = self.core.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(d);
                    core.finish_all();
                });
            }
            Ok(())
        }
        fn stop(&self) -> Result<()> {
            self.core.stop()
        }
        fn running(&self) -> bool {
            self.core.running()
        }
        fn back_to_home(&self) -> Result<()> {
            self.core.back_to_home()
        }
        fn screencap(&self) -> Result<()> {
            self.core.screencap()
        }
        fn image_png(&self) -> Result<Option<Vec<u8>>> {
            self.core.image_png()
        }
        fn image_bgr(&self) -> Result<Option<Vec<u8>>> {
            self.core.image_bgr()
        }
        fn click(&self, x: i32, y: i32) -> Result<()> {
            self.core.click(x, y)
        }
        fn uuid(&self) -> Option<String> {
            self.core.uuid()
        }
    }
}
