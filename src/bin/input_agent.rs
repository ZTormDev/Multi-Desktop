//! Connects only to the private Gamescope libei socket from one desktop session.
use std::{
    collections::BTreeSet,
    env,
    ffi::CString,
    fs, io,
    io::Read,
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::Path,
    ptr, thread,
    time::Duration,
};

use multi_desktop::transport::InputEvent;

fn main() -> io::Result<()> {
    let runtime = env::var("MULTIDESKTOP_DESKTOP_RUNTIME")
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "missing desktop runtime"))?;
    let status = Path::new(&runtime).join("input-status");
    let Some(socket) = env::var("LIBEI_SOCKET")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        return write_status(
            &status,
            "state=unavailable\nreason=gamescope-did-not-export-libei-socket\n",
        );
    };
    if !is_socket(&socket_path(&socket)) {
        return write_status(&status, "state=waiting\nbackend=libei\n");
    }
    match connect_libei(&socket) {
        Ok(context) => run_libei(context, &status, Path::new(&runtime)),
        Err(error) => write_status(
            &status,
            &format!(
                "state=error\nbackend=libei\nreason={}\n",
                sanitize(&error.to_string())
            ),
        ),
    }
}

fn connect_libei(socket: &str) -> io::Result<*mut ei> {
    let socket = CString::new(socket)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid libei socket path"))?;
    // SAFETY: all pointers come from libei, stay valid for this scope, and the
    // library owns the context after successful setup.
    unsafe {
        let context = ei_new_sender(ptr::null_mut());
        if context.is_null() {
            return Err(io::Error::other("could not create libei sender"));
        }
        ei_configure_name(context, c"multi-desktop-input-agent".as_ptr());
        if ei_setup_backend_socket(context, socket.as_ptr()) < 0 {
            ei_unref(context);
            return Err(io::Error::other(
                "Gamescope rejected the private libei connection",
            ));
        }
        for _ in 0..20 {
            ei_dispatch(context);
            let mut event = ei_get_event(context);
            while !event.is_null() {
                let event_type = ei_event_get_type(event);
                ei_event_unref(event);
                if event_type == EI_EVENT_CONNECT {
                    return Ok(context);
                }
                if event_type == EI_EVENT_DISCONNECT {
                    ei_unref(context);
                    return Err(io::Error::other("Gamescope disconnected libei"));
                }
                event = ei_get_event(context);
            }
            thread::sleep(Duration::from_millis(100));
        }
        ei_unref(context);
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "timed out waiting for Gamescope libei approval",
    ))
}

struct DeviceState {
    device: *mut ei_device,
    capabilities: i32,
    resumed: bool,
    emulating: bool,
    sequence: u32,
    keys: BTreeSet<u16>,
    buttons: BTreeSet<u16>,
}

impl DeviceState {
    fn has(&self, capability: i32) -> bool {
        self.capabilities & capability != 0
    }

    fn start_emulating(&mut self) {
        if self.resumed && !self.emulating {
            self.sequence = self.sequence.wrapping_add(1).max(1);
            // SAFETY: the retained device is resumed and belongs to this
            // sender context.
            unsafe { ei_device_start_emulating(self.device, self.sequence) };
            self.emulating = true;
        }
    }
}

struct InputClient {
    stream: UnixStream,
    pending: Vec<u8>,
}

impl InputClient {
    fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            pending: Vec::new(),
        })
    }

    fn receive(&mut self) -> io::Result<Vec<InputEvent>> {
        let mut chunk = [0_u8; 256];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "input channel disconnected",
                    ));
                }
                Ok(count) => {
                    self.pending.extend_from_slice(&chunk[..count]);
                    if self.pending.len() > 4096 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "input buffer limit exceeded",
                        ));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        let mut events = Vec::new();
        while let Some(tag) = self.pending.first().copied() {
            let length = InputEvent::encoded_len_for_tag(tag)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unknown input event"))?;
            if self.pending.len() < length {
                break;
            }
            let payload: Vec<_> = self.pending.drain(..length).collect();
            events.push(InputEvent::decode(&payload)?);
        }
        Ok(events)
    }
}

fn run_libei(context: *mut ei, status: &Path, runtime: &Path) -> io::Result<()> {
    let socket = runtime.join("input-events.sock");
    remove_stale_socket(&socket)?;
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let mut devices: Vec<DeviceState> = Vec::new();
    let mut client: Option<InputClient> = None;
    let mut published_state = (false, false);
    write_status(
        status,
        "state=connected\nbackend=libei\nseat=gamescope-private\nchannel=waiting\n",
    )?;
    loop {
        // SAFETY: `context` remains owned by this agent until the disconnect
        // event or the process exits.
        unsafe {
            ei_dispatch(context);
            let mut event = ei_get_event(context);
            while !event.is_null() {
                let event_type = ei_event_get_type(event);
                if event_type == EI_EVENT_SEAT_ADDED {
                    let seat = ei_event_get_seat(event);
                    if !seat.is_null() {
                        ei_seat_bind_capabilities(
                            seat,
                            EI_DEVICE_CAP_KEYBOARD,
                            EI_DEVICE_CAP_POINTER,
                            EI_DEVICE_CAP_BUTTON,
                            EI_DEVICE_CAP_SCROLL,
                            ptr::null::<std::ffi::c_void>(),
                        );
                    }
                }
                if event_type == EI_EVENT_DEVICE_ADDED {
                    let device = ei_event_get_device(event);
                    if !device.is_null() {
                        let capabilities = device_capabilities(device);
                        if capabilities != 0 {
                            devices.push(DeviceState {
                                device: ei_device_ref(device),
                                capabilities,
                                resumed: false,
                                emulating: false,
                                sequence: 0,
                                keys: BTreeSet::new(),
                                buttons: BTreeSet::new(),
                            });
                        }
                    }
                }
                if matches!(
                    event_type,
                    EI_EVENT_DEVICE_REMOVED | EI_EVENT_DEVICE_PAUSED | EI_EVENT_DEVICE_RESUMED
                ) {
                    let device = ei_event_get_device(event);
                    if let Some(index) = devices.iter().position(|state| state.device == device) {
                        if event_type == EI_EVENT_DEVICE_REMOVED {
                            let removed = devices.remove(index);
                            ei_device_unref(removed.device);
                        } else if event_type == EI_EVENT_DEVICE_PAUSED {
                            devices[index].resumed = false;
                            devices[index].emulating = false;
                            devices[index].keys.clear();
                            devices[index].buttons.clear();
                        } else {
                            devices[index].resumed = true;
                            if client.is_some() {
                                devices[index].start_emulating();
                            }
                        }
                    }
                }
                let disconnected = event_type == EI_EVENT_DISCONNECT;
                ei_event_unref(event);
                if disconnected {
                    for state in devices.drain(..) {
                        ei_device_unref(state.device);
                    }
                    ei_unref(context);
                    return write_status(status, "state=disconnected\nbackend=libei\n");
                }
                event = ei_get_event(context);
            }
        }

        let ready = required_capabilities_ready(&devices);
        if ready {
            if client.is_none() {
                match listener.accept() {
                    Ok((stream, _)) => {
                        client = Some(InputClient::new(stream)?);
                        start_devices(&mut devices);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error),
                }
            } else {
                while let Ok((extra, _)) = listener.accept() {
                    drop(extra);
                }
            }
        }
        let events = match client.as_mut().map(InputClient::receive) {
            Some(Ok(events)) => events,
            Some(Err(_)) => {
                reset_devices(context, &mut devices);
                client = None;
                Vec::new()
            }
            None => Vec::new(),
        };
        for event in events {
            if event == InputEvent::Reset {
                reset_devices(context, &mut devices);
            } else if apply_event(context, &mut devices, event).is_err() {
                reset_devices(context, &mut devices);
                client = None;
                break;
            }
        }
        let current_state = (ready, client.is_some());
        if current_state != published_state {
            publish_ready_status(status, current_state.0, current_state.1)?;
            published_state = current_state;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn device_capabilities(device: *mut ei_device) -> i32 {
    [
        EI_DEVICE_CAP_KEYBOARD,
        EI_DEVICE_CAP_POINTER,
        EI_DEVICE_CAP_BUTTON,
        EI_DEVICE_CAP_SCROLL,
    ]
    .into_iter()
    .filter(|capability| {
        // SAFETY: device comes from the current libei event.
        unsafe { ei_device_has_capability(device, *capability) }
    })
    .fold(0, |capabilities, capability| capabilities | capability)
}

fn required_capabilities_ready(devices: &[DeviceState]) -> bool {
    [
        EI_DEVICE_CAP_KEYBOARD,
        EI_DEVICE_CAP_POINTER,
        EI_DEVICE_CAP_BUTTON,
        EI_DEVICE_CAP_SCROLL,
    ]
    .into_iter()
    .all(|capability| {
        devices
            .iter()
            .any(|device| device.resumed && device.has(capability))
    })
}

fn start_devices(devices: &mut [DeviceState]) {
    for device in devices {
        device.start_emulating();
    }
}

fn apply_event(context: *mut ei, devices: &mut [DeviceState], event: InputEvent) -> io::Result<()> {
    let capability = match event {
        InputEvent::Key { .. } => EI_DEVICE_CAP_KEYBOARD,
        InputEvent::PointerMotion { .. } => EI_DEVICE_CAP_POINTER,
        InputEvent::PointerButton { .. } => EI_DEVICE_CAP_BUTTON,
        InputEvent::Scroll { .. } => EI_DEVICE_CAP_SCROLL,
        InputEvent::Reset => return Ok(()),
    };
    let device = devices
        .iter_mut()
        .find(|device| device.resumed && device.has(capability))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "input capability unavailable")
        })?;
    device.start_emulating();
    // SAFETY: the selected device is retained, resumed, emulating and exposes
    // the capability required by this validated event.
    unsafe {
        match event {
            InputEvent::Key { code, pressed } => {
                if pressed == device.keys.contains(&code) {
                    return Ok(());
                }
                ei_device_keyboard_key(device.device, u32::from(code), pressed);
                if pressed {
                    device.keys.insert(code);
                } else {
                    device.keys.remove(&code);
                }
            }
            InputEvent::PointerMotion { x, y } => {
                if x == 0 && y == 0 {
                    return Ok(());
                }
                ei_device_pointer_motion(device.device, f64::from(x), f64::from(y));
            }
            InputEvent::PointerButton { code, pressed } => {
                if pressed == device.buttons.contains(&code) {
                    return Ok(());
                }
                ei_device_button_button(device.device, u32::from(code), pressed);
                if pressed {
                    device.buttons.insert(code);
                } else {
                    device.buttons.remove(&code);
                }
            }
            InputEvent::Scroll { x, y } => {
                if x == 0 && y == 0 {
                    return Ok(());
                }
                ei_device_scroll_discrete(device.device, i32::from(x), i32::from(y));
            }
            InputEvent::Reset => return Ok(()),
        }
        ei_device_frame(device.device, ei_now(context));
    }
    Ok(())
}

fn reset_devices(context: *mut ei, devices: &mut [DeviceState]) {
    for device in devices {
        if !device.resumed || !device.emulating {
            device.keys.clear();
            device.buttons.clear();
            continue;
        }
        let keys: Vec<_> = device.keys.iter().copied().collect();
        let buttons: Vec<_> = device.buttons.iter().copied().collect();
        // SAFETY: the retained device is still resumed and emulating. Release
        // all logical state before ending the transaction.
        unsafe {
            for code in keys {
                ei_device_keyboard_key(device.device, u32::from(code), false);
            }
            for code in buttons {
                ei_device_button_button(device.device, u32::from(code), false);
            }
            if !device.keys.is_empty() || !device.buttons.is_empty() {
                ei_device_frame(device.device, ei_now(context));
            }
            ei_device_stop_emulating(device.device);
        }
        device.keys.clear();
        device.buttons.clear();
        device.emulating = false;
    }
}

fn publish_ready_status(status: &Path, ready: bool, active: bool) -> io::Result<()> {
    if ready {
        write_status(
            status,
            &format!(
                "state=ready\nbackend=libei\nseat=gamescope-private\nkeyboard=ready\npointer=ready\nbutton=ready\nscroll=ready\nchannel={}\n",
                if active { "active" } else { "waiting" }
            ),
        )
    } else {
        write_status(
            status,
            "state=connected\nbackend=libei\nseat=gamescope-private\nchannel=waiting\n",
        )
    }
}

fn remove_stale_socket(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "input endpoint path exists and is not a socket",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn is_socket(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character != '\n' && *character != '\r')
        .collect()
}

#[allow(non_camel_case_types)]
enum ei {}
#[allow(non_camel_case_types)]
enum ei_event {}
#[allow(non_camel_case_types)]
enum ei_seat {}
#[allow(non_camel_case_types)]
enum ei_device {}
const EI_EVENT_CONNECT: i32 = 1;
const EI_EVENT_DISCONNECT: i32 = 2;
const EI_EVENT_SEAT_ADDED: i32 = 3;
const EI_EVENT_DEVICE_ADDED: i32 = 5;
const EI_EVENT_DEVICE_REMOVED: i32 = 6;
const EI_EVENT_DEVICE_PAUSED: i32 = 7;
const EI_EVENT_DEVICE_RESUMED: i32 = 8;
const EI_DEVICE_CAP_POINTER: i32 = 1;
const EI_DEVICE_CAP_KEYBOARD: i32 = 4;
const EI_DEVICE_CAP_SCROLL: i32 = 16;
const EI_DEVICE_CAP_BUTTON: i32 = 32;

#[link(name = "ei")]
unsafe extern "C" {
    fn ei_new_sender(user_data: *mut std::ffi::c_void) -> *mut ei;
    fn ei_unref(context: *mut ei) -> *mut ei;
    fn ei_configure_name(context: *mut ei, name: *const std::ffi::c_char);
    fn ei_setup_backend_socket(context: *mut ei, socket: *const std::ffi::c_char) -> i32;
    fn ei_dispatch(context: *mut ei);
    fn ei_get_event(context: *mut ei) -> *mut ei_event;
    fn ei_event_get_type(event: *mut ei_event) -> i32;
    fn ei_event_get_seat(event: *mut ei_event) -> *mut ei_seat;
    fn ei_event_get_device(event: *mut ei_event) -> *mut ei_device;
    fn ei_seat_bind_capabilities(seat: *mut ei_seat, ...);
    fn ei_device_has_capability(device: *mut ei_device, capability: i32) -> bool;
    fn ei_device_ref(device: *mut ei_device) -> *mut ei_device;
    fn ei_device_unref(device: *mut ei_device) -> *mut ei_device;
    fn ei_device_start_emulating(device: *mut ei_device, sequence: u32);
    fn ei_device_stop_emulating(device: *mut ei_device);
    fn ei_device_keyboard_key(device: *mut ei_device, keycode: u32, is_press: bool);
    fn ei_device_pointer_motion(device: *mut ei_device, x: f64, y: f64);
    fn ei_device_button_button(device: *mut ei_device, button: u32, is_press: bool);
    fn ei_device_scroll_discrete(device: *mut ei_device, x: i32, y: i32);
    fn ei_device_frame(device: *mut ei_device, time: u64);
    fn ei_now(context: *mut ei) -> u64;
    fn ei_event_unref(event: *mut ei_event) -> *mut ei_event;
}

fn socket_path(socket: &str) -> std::path::PathBuf {
    if socket.starts_with('/') {
        socket.into()
    } else {
        env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
            .join(socket)
    }
}

fn write_status(path: &Path, contents: &str) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn input_client_handles_fragmented_and_combined_events() {
        let (mut sender, receiver) = UnixStream::pair().unwrap();
        let mut client = InputClient::new(receiver).unwrap();
        let key = InputEvent::Key {
            code: 30,
            pressed: true,
        }
        .encode();
        sender.write_all(&key[..2]).unwrap();
        assert!(client.receive().unwrap().is_empty());
        sender.write_all(&key[2..]).unwrap();
        sender.write_all(&InputEvent::Reset.encode()).unwrap();
        assert_eq!(
            client.receive().unwrap(),
            vec![
                InputEvent::Key {
                    code: 30,
                    pressed: true
                },
                InputEvent::Reset
            ]
        );
    }
}
