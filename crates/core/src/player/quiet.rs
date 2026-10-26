//! A helper program run to its end, out of sight. On Windows a program started the usual way turns
//! the pointer into "app starting" until it shows a window, and mpv making a thumbnail never does:
//! seek previews kept the pointer busy all through an episode.

use std::path::Path;

#[cfg(not(windows))]
pub async fn run(program: &Path, args: Vec<String>) -> std::io::Result<()> {
    tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map(|_| ())
}

#[cfg(windows)]
pub async fn run(program: &Path, args: Vec<String>) -> std::io::Result<()> {
    let line = std::iter::once(program.to_string_lossy().into_owned())
        .chain(args)
        .map(|a| quote(&a))
        .collect::<Vec<_>>()
        .join(" ");
    tokio::task::spawn_blocking(move || win::run(&line))
        .await
        .map_err(std::io::Error::other)?
}

/// One argument as the C runtime reads it back: in quotes when it has spaces or quotes, with the
/// backslashes before a quote doubled.
#[cfg_attr(not(windows), allow(dead_code))]
fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut slashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => {
                slashes += 1;
                continue;
            }
            '"' => out.extend(std::iter::repeat_n('\\', slashes * 2 + 1)),
            _ => out.extend(std::iter::repeat_n('\\', slashes)),
        }
        slashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    const STARTF_FORCEOFFFEEDBACK: u32 = 0x80;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const INFINITE: u32 = 0xFFFF_FFFF;

    #[repr(C)]
    struct StartupInfo {
        cb: u32,
        reserved: *mut u16,
        desktop: *mut u16,
        title: *mut u16,
        x: u32,
        y: u32,
        x_size: u32,
        y_size: u32,
        x_count_chars: u32,
        y_count_chars: u32,
        fill_attribute: u32,
        flags: u32,
        show_window: u16,
        reserved2_size: u16,
        reserved2: *mut u8,
        std_input: *mut c_void,
        std_output: *mut c_void,
        std_error: *mut c_void,
    }

    #[repr(C)]
    struct ProcessInfo {
        process: *mut c_void,
        thread: *mut c_void,
        process_id: u32,
        thread_id: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateProcessW(
            application: *const u16,
            command_line: *mut u16,
            process_attributes: *const c_void,
            thread_attributes: *const c_void,
            inherit_handles: i32,
            creation_flags: u32,
            environment: *const c_void,
            current_directory: *const u16,
            startup_info: *const StartupInfo,
            process_info: *mut ProcessInfo,
        ) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, ms: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    pub fn run(line: &str) -> std::io::Result<()> {
        let mut line: Vec<u16> = line.encode_utf16().chain([0]).collect();
        let mut startup: StartupInfo = unsafe { std::mem::zeroed() };
        startup.cb = size_of::<StartupInfo>() as u32;
        startup.flags = STARTF_FORCEOFFFEEDBACK;
        let mut info: ProcessInfo = unsafe { std::mem::zeroed() };
        let made = unsafe {
            CreateProcessW(
                std::ptr::null(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                CREATE_NO_WINDOW,
                std::ptr::null(),
                std::ptr::null(),
                &startup,
                &mut info,
            )
        };
        if made == 0 {
            return Err(std::io::Error::last_os_error());
        }
        unsafe {
            WaitForSingleObject(info.process, INFINITE);
            CloseHandle(info.thread);
            CloseHandle(info.process);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn arguments_survive_the_command_line() {
        assert_eq!(quote("--frames=1"), "--frames=1");
        assert_eq!(quote(r"C:\Users\Jo Do\a.ts"), r#""C:\Users\Jo Do\a.ts""#);
        assert_eq!(quote(r"C:\dir with space\"), r#""C:\dir with space\\""#);
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote(""), r#""""#);
    }
}
