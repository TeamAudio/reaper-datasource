use crate::parse::{self, ParseOptions};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

static LOW: OnceLock<reaper_low::Reaper> = OnceLock::new();

pub fn init(low: reaper_low::Reaper) {
    let _ = LOW.set(low);
}

unsafe fn string_arg(args: &[*mut c_void], index: usize) -> Result<&str, String> {
    let ptr = args
        .get(index)
        .copied()
        .filter(|ptr| !ptr.is_null())
        .ok_or_else(|| format!("missing argument {}", index + 1))?;
    CStr::from_ptr(ptr.cast::<c_char>())
        .to_str()
        .map_err(|_| format!("argument {} is not UTF-8", index + 1))
}

unsafe fn args<'a>(arglist: *mut *mut c_void, count: c_int) -> &'a [*mut c_void] {
    if arglist.is_null() || count <= 0 {
        &[]
    } else {
        std::slice::from_raw_parts(arglist, count as usize)
    }
}

/// An output-string parameter pair (char* buffer + int size) as received
/// through the vararg ABI. Integer arguments arrive as the pointer value
/// itself, not a pointer to the integer.
struct OutBuffer {
    ptr: *mut c_char,
    size: c_int,
}

impl OutBuffer {
    unsafe fn from_args(args: &[*mut c_void], buffer_index: usize) -> Self {
        Self {
            ptr: args.get(buffer_index).copied().unwrap_or(std::ptr::null_mut()) as *mut c_char,
            size: args
                .get(buffer_index + 1)
                .copied()
                .map(|value| value as isize as c_int)
                .unwrap_or(0),
        }
    }

    /// Writes a NUL-terminated string into the buffer, growing it through
    /// REAPER's realloc_cmd_ptr when possible (valid only while inside an
    /// API call from a NeedBig parameter; native C callers get plain
    /// truncation-as-failure instead).
    unsafe fn write(&mut self, value: &str) -> bool {
        self.write_with(value, |ptr, size, needed| {
            LOW.get()
                .is_some_and(|low| low.realloc_cmd_ptr(ptr, size, needed))
        })
    }

    unsafe fn write_with(
        &mut self,
        value: &str,
        grow: impl FnOnce(*mut *mut c_char, *mut c_int, c_int) -> bool,
    ) -> bool {
        let clean = value.replace('\0', "\u{fffd}");
        if clean.len() >= c_int::MAX as usize {
            return false;
        }
        let length = clean.len() as c_int;
        if length < self.size {
            // Fits in the caller's buffer, which REAPER reads as a C string.
            if self.ptr.is_null() {
                return false;
            }
            std::ptr::copy_nonoverlapping(clean.as_ptr(), self.ptr as *mut u8, clean.len());
            *self.ptr.add(clean.len()) = 0;
            return true;
        }
        // Grow to the exact content length: REAPER returns the whole grown
        // buffer as the string, so a NUL terminator would come back as a
        // trailing byte.
        if !grow(&mut self.ptr, &mut self.size, length) {
            return false;
        }
        if self.ptr.is_null() || self.size < length {
            return false;
        }
        std::ptr::copy_nonoverlapping(clean.as_ptr(), self.ptr as *mut u8, clean.len());
        true
    }
}

/// Runs an API implementation and reports its outcome REAPER-idiomatically:
/// retval is true and the out-buffer holds the result on success; retval is
/// false and the out-buffer holds an error message on failure. Panics are
/// caught so they never cross the extern "C" boundary.
unsafe fn respond(
    mut out: OutBuffer,
    work: impl FnOnce() -> Result<String, String>,
) -> *mut c_void {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let result = work().and_then(|value| {
            if out.write(&value) {
                Ok(())
            } else {
                Err("result does not fit in the output buffer".to_owned())
            }
        });
        match result {
            Ok(()) => true,
            Err(error) => {
                out.write(&error);
                false
            }
        }
    }))
    .unwrap_or(false);
    outcome as isize as *mut c_void
}

pub unsafe extern "C" fn parse_vararg(arglist: *mut *mut c_void, count: c_int) -> *mut c_void {
    let args = args(arglist, count);
    respond(OutBuffer::from_args(args, 2), || {
        let path = string_arg(args, 0)?;
        let options_json = string_arg(args, 1).unwrap_or("");
        let options = if options_json.trim().is_empty() {
            ParseOptions::default()
        } else {
            serde_json::from_str::<ParseOptions>(options_json)
                .map_err(|error| format!("invalid options_json: {error}"))?
        };
        parse::parse_workbook(path, &options).map(|value| value.to_string())
    })
}

pub unsafe extern "C" fn list_sheets_vararg(
    arglist: *mut *mut c_void,
    count: c_int,
) -> *mut c_void {
    let args = args(arglist, count);
    respond(OutBuffer::from_args(args, 1), || {
        let path = string_arg(args, 0)?;
        parse::list_sheets(path).map(|value| value.to_string())
    })
}

pub extern "C" fn parse_native(
    file_path: *const c_char,
    options_json: *const c_char,
    result_out: *mut c_char,
    result_out_sz: c_int,
) -> bool {
    let args = [
        file_path as *mut c_void,
        options_json as *mut c_void,
        result_out as *mut c_void,
        result_out_sz as isize as *mut c_void,
    ];
    unsafe { !parse_vararg(args.as_ptr() as *mut *mut c_void, args.len() as c_int).is_null() }
}

pub extern "C" fn list_sheets_native(
    file_path: *const c_char,
    result_out: *mut c_char,
    result_out_sz: c_int,
) -> bool {
    let args = [
        file_path as *mut c_void,
        result_out as *mut c_void,
        result_out_sz as isize as *mut c_void,
    ];
    unsafe {
        !list_sheets_vararg(args.as_ptr() as *mut *mut c_void, args.len() as c_int).is_null()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn write_to(
        buffer: &mut [u8],
        value: &str,
        grow: impl FnOnce(*mut *mut c_char, *mut c_int, c_int) -> bool,
    ) -> (bool, OutBuffer) {
        let mut out = OutBuffer {
            ptr: buffer.as_mut_ptr() as *mut c_char,
            size: buffer.len() as c_int,
        };
        let ok = out.write_with(value, grow);
        (ok, out)
    }

    fn contents(out: &OutBuffer) -> String {
        unsafe { CStr::from_ptr(out.ptr).to_string_lossy().into_owned() }
    }

    #[test]
    fn writes_when_the_value_fits() {
        let mut buffer = [0u8; 16];
        let (ok, out) = unsafe { write_to(&mut buffer, "hello", |_, _, _| false) };
        assert!(ok);
        assert_eq!(contents(&out), "hello");
    }

    #[test]
    fn replaces_interior_nuls() {
        let mut buffer = [0u8; 16];
        let (ok, out) = unsafe { write_to(&mut buffer, "a\0b", |_, _, _| false) };
        assert!(ok);
        assert_eq!(contents(&out), "a\u{fffd}b");
    }

    #[test]
    fn fails_without_growth_when_too_small() {
        let mut buffer = [0u8; 4];
        let (ok, _) = unsafe { write_to(&mut buffer, "too long to fit", |_, _, _| false) };
        assert!(!ok);
    }

    #[test]
    fn grows_to_the_exact_length_without_nul() {
        let mut small = [0u8; 4];
        let mut big = vec![0xffu8; 64];
        let big_ptr = big.as_mut_ptr() as *mut c_char;
        let (ok, out) = unsafe {
            write_to(&mut small, "too long to fit", |ptr, size, needed| {
                // Exact content length, no room reserved for a terminator —
                // REAPER returns the whole grown buffer as the string.
                assert_eq!(needed, 15);
                *ptr = big_ptr;
                *size = needed;
                true
            })
        };
        assert!(ok);
        assert_eq!(out.size, 15);
        assert_eq!(&big[..15], b"too long to fit");
        assert_eq!(big[15], 0xff, "no NUL terminator written past the content");
    }

    #[test]
    fn exact_fit_without_grow_requires_terminator_space() {
        // A 5-byte buffer cannot hold "hello" as a C string; the write must
        // go through grow rather than silently dropping the terminator.
        let mut buffer = [0xffu8; 5];
        let (ok, _) = unsafe { write_to(&mut buffer, "hello", |_, _, _| false) };
        assert!(!ok);
    }

    #[test]
    fn null_buffer_fails_cleanly() {
        let mut out = OutBuffer {
            ptr: std::ptr::null_mut(),
            size: 0,
        };
        let ok = unsafe { out.write_with("anything", |_, _, _| false) };
        assert!(!ok);
    }
}
