//! Thin, self-closing wrappers over the handful of Win32 registry reads the
//! Steam and GOG connectors need. Kept in one place so the `unsafe` is
//! auditable and the launcher modules stay declarative. Every read fails soft
//! to `None` — a launcher that isn't installed just yields no key.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, KEY_READ,
};

/// UTF-16, NUL-terminated — what the `*W` registry APIs expect.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// An opened registry key that closes itself on drop.
pub struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

impl Key {
    /// Open `root\subkey` for reading. `None` if the key doesn't exist.
    pub fn open(root: HKEY, subkey: &str) -> Option<Key> {
        let subkey = wide(subkey);
        let mut hkey = HKEY::default();
        let status =
            unsafe { RegOpenKeyExW(root, PCWSTR(subkey.as_ptr()), Some(0), KEY_READ, &mut hkey) };
        (status == ERROR_SUCCESS).then_some(Key(hkey))
    }

    /// Read a `REG_DWORD` value. `None` if missing.
    pub fn dword(&self, value: &str) -> Option<u32> {
        let name = wide(value);
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                PCWSTR(name.as_ptr()),
                None,
                None,
                Some(std::ptr::from_mut(&mut data).cast::<u8>()),
                Some(&mut size),
            )
        };
        (status == ERROR_SUCCESS).then_some(data)
    }

    /// Read a `REG_SZ` string value. `None` if missing or empty.
    pub fn string(&self, value: &str) -> Option<String> {
        let name = wide(value);
        // First call probes the byte size; second reads into a right-sized buf.
        let mut size: u32 = 0;
        let status = unsafe {
            RegQueryValueExW(self.0, PCWSTR(name.as_ptr()), None, None, None, Some(&mut size))
        };
        if status != ERROR_SUCCESS || size == 0 {
            return None;
        }
        // `size` is bytes; registry strings are UTF-16.
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        // `size` (already mut) is reused as the second call's in/out length.
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                PCWSTR(name.as_ptr()),
                None,
                None,
                Some(buf.as_mut_ptr().cast::<u8>()),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        // Trim at the stored NUL terminator.
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let s = String::from_utf16_lossy(&buf[..end]);
        (!s.is_empty()).then_some(s)
    }

    /// Names of this key's immediate subkeys (e.g. the GOG game ids).
    pub fn subkey_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut index = 0u32;
        loop {
            // Max registry key-name length is 255 chars; +1 for the NUL.
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    Some(PWSTR(buf.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if status == ERROR_SUCCESS {
                names.push(String::from_utf16_lossy(&buf[..len as usize]));
            } else if status != ERROR_MORE_DATA {
                // ERROR_NO_MORE_ITEMS (done) or any hard error: stop. A name
                // longer than our 255-char buffer (ERROR_MORE_DATA) is just
                // skipped — game-id keys are always well under that.
                break;
            }
            index += 1;
        }
        names
    }
}
