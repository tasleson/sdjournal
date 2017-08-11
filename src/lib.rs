// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate libc;

use libc::{c_int, c_void, size_t};

use std::ffi::CString;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::slice;
use std::u64;
use std::fmt;

// Opaque data type for journal handle for use in ffi calls
pub enum SdJournal {}

enum SdJournalOpen {
    LocalOnly = 1 << 0,

    // The following are not being utilized at the moment, just here for documentation
    /*
    RuntimeOnly = 1 << 1,
    System = 1 << 2,
    CurrentUser = 1 << 3,
    OsRoot = 1 << 4,
    */
}

#[derive(Debug, Clone)]
pub struct ClibraryError {
    pub message: String,
    pub return_code: c_int,
    pub err_reason: String,
}

impl fmt::Display for ClibraryError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{} (rc={}, errno msg={})",
               self.message, self.return_code, self.err_reason)
    }
}

impl std::error::Error for ClibraryError {
    fn description(&self) -> &str {
        &self.message
    }
}

// Wakeup event types
enum SdJournalWait {
    Nop = 0,
    Append = 1,
    Invalidate = 2,
}

#[link(name = "systemd")]
extern {
    fn sd_journal_open(ret: *mut *mut SdJournal, flags: c_int) -> c_int;
    fn sd_journal_next(j: *mut SdJournal) -> c_int;
    fn sd_journal_get_data(j: *mut SdJournal,
                           field: *const c_char,
                           data: *mut *mut c_void,
                           length: *mut size_t) -> c_int;
    fn sd_journal_close(j: *mut SdJournal);
    fn sd_journal_wait(j: *mut SdJournal, timeout_usec: u64) -> c_int;
}

// Copied and pasted from https://github.com/rust-lang/rust/blob/master/src/libstd/sys/unix/os.rs
// if I can figure out how to call it I will delete this!!!
pub fn error_string(errno: i32) -> String {
    extern {
        #[cfg_attr(any(target_os = "linux", target_env = "newlib"),
                   link_name = "__xpg_strerror_r")]
        fn strerror_r(errnum: c_int, buf: *mut c_char,
                      buflen: libc::size_t) -> c_int;
    }

    let mut buf = [0 as c_char; 128];

    let p = buf.as_mut_ptr();
    unsafe {
        if strerror_r(errno as c_int, p, buf.len()) < 0 {
            panic!("strerror_r failure");
        }

        let p = p as *const _;
        std::str::from_utf8(CStr::from_ptr(p).to_bytes()).unwrap().to_owned()
    }
}

pub struct Journal {
    handle: *mut SdJournal,
    pub timeout_us: u64
}

// Close the handle when we go out of scope and get cleaned up
impl Drop for Journal {
    fn drop(&mut self) {
        unsafe { sd_journal_close(self.handle) };
    }
}

impl Journal {
    pub fn new() -> Result<Journal, ClibraryError> {
        let mut tmp_handle = 0 as *mut SdJournal;

        let rc = unsafe {
            sd_journal_open((&mut tmp_handle) as *mut _ as *mut *mut SdJournal,
                            SdJournalOpen::LocalOnly as c_int)
        };
        if rc != 0 {
            Err(ClibraryError {
                message: String::from("Error on sd_journal_open"),
                return_code: rc,
                err_reason: error_string(-rc),
            })
        } else {
            Ok(Journal { handle: tmp_handle, timeout_us: std::u64::MAX })
        }
    }

    fn get_log_entry(&mut self) -> Result<String, ClibraryError> {
        let mut x = 0 as *mut c_void;
        let mut len = 0 as size_t;
        let field = CString::new("MESSAGE").unwrap();

        let log_msg: String;
        let rc = unsafe {
            sd_journal_get_data(self.handle, field.as_ptr(),
                                (&mut x) as *mut _ as *mut *mut c_void,
                                &mut len)
        };
        if rc == 0 {
            let slice = unsafe { slice::from_raw_parts(x as *const u8, len) };
            log_msg = String::from_utf8(slice[8..len].to_vec()).unwrap();
        } else {
            return Err(ClibraryError {
                message: String::from("Error on sd_journal_get_data"),
                return_code: rc,
                err_reason: error_string(-rc),
            });
        }

        Ok(log_msg)
    }
}

impl Iterator for Journal {
    type Item = Result<String, ClibraryError>;

    fn next(&mut self) -> Option<Result<String, ClibraryError>> {
        // Hit the iterator, if we have something return it, else try waiting for it
        let log_msg: String;

        loop {
            let log_entry = unsafe { sd_journal_next(self.handle) };
            if log_entry < 0 {
                return Some(Err(ClibraryError {
                    message: String::from("Error on sd_journal_next"),
                    return_code: log_entry,
                    err_reason: error_string(-log_entry),
                }));
            }

            if log_entry == 0 {
                // TODO: Figure out how to make a match work when comparing int to enum type.
                let wait_rc = unsafe { sd_journal_wait(self.handle, self.timeout_us) };

                if wait_rc == SdJournalWait::Nop as i32 {
                    return None;
                } else if wait_rc == SdJournalWait::Append as i32 ||
                    wait_rc == SdJournalWait::Invalidate as i32 {
                    continue;
                } else {
                    return Some(Err(ClibraryError {
                        message: String::from("Error on sd_journal_wait"),
                        return_code: log_entry,
                        err_reason: error_string(-wait_rc),
                    }));
                }
            }

            //TODO: Propagate the error instead of doing a panic here!
            log_msg = self.get_log_entry().expect("Failed to get next log entry");
            break;
        }

        Some(Ok(log_msg))
    }
}