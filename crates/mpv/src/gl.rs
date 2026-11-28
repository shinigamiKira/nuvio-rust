//! Resolves OpenGL entry points for libmpv.

use std::ffi::{CStr, c_char, c_void};

#[cfg(windows)]
mod imp {
    //! opengl32.dll only exports the OpenGL 1.1 functions; everything newer has to come from `wglGetProcAddress` while
    //! the context is current.

    use std::ffi::{CStr, c_char, c_void};
    use std::sync::OnceLock;

    use libloading::Library;

    type WglGetProcAddress = unsafe extern "system" fn(*const c_char) -> *mut c_void;

    struct OpenGl {
        library: Library,
        wgl_get_proc_address: WglGetProcAddress,
    }

    fn opengl() -> Option<&'static OpenGl> {
        static GL: OnceLock<Option<OpenGl>> = OnceLock::new();
        GL.get_or_init(|| {
            // SAFETY: opengl32.dll is a system library; the symbol has the documented signature.
            unsafe {
                let library = Library::new("opengl32.dll").ok()?;
                let wgl_get_proc_address = *library.get::<WglGetProcAddress>(b"wglGetProcAddress\0").ok()?;
                Some(OpenGl { library, wgl_get_proc_address })
            }
        })
        .as_ref()
    }

    /// Some drivers return 1, 2, 3 or -1 instead of null for functions they do not have.
    fn is_valid(pointer: *mut c_void) -> bool {
        !matches!(pointer as isize, 0 | 1 | 2 | 3 | -1)
    }

    pub fn proc_address(name: &CStr) -> *mut c_void {
        let Some(gl) = opengl() else { return std::ptr::null_mut() };
        // SAFETY: `name` is NUL terminated.
        unsafe {
            let pointer = (gl.wgl_get_proc_address)(name.as_ptr());
            if is_valid(pointer) {
                return pointer;
            }
            gl.library
                .get::<*mut c_void>(name.to_bytes_with_nul())
                .map(|symbol| *symbol)
                .unwrap_or(std::ptr::null_mut())
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    //! Every function the system's OpenGL has is exported by the framework itself.

    use std::ffi::{CStr, c_void};
    use std::sync::OnceLock;

    use libloading::Library;

    const FRAMEWORK: &str = "/System/Library/Frameworks/OpenGL.framework/OpenGL";

    pub fn proc_address(name: &CStr) -> *mut c_void {
        static GL: OnceLock<Option<Library>> = OnceLock::new();
        // SAFETY: a system framework, whose initialisers are the normal ones.
        let Some(library) = GL.get_or_init(|| unsafe { Library::new(FRAMEWORK) }.ok()) else { return std::ptr::null_mut() };
        // SAFETY: the symbol is only handed on as an address.
        unsafe { library.get::<*mut c_void>(name.to_bytes_with_nul()).map(|symbol| *symbol).unwrap_or(std::ptr::null_mut()) }
    }
}

pub fn proc_address(name: &CStr) -> *mut c_void {
    imp::proc_address(name)
}

pub(crate) unsafe extern "C" fn loader(_context: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: libmpv passes a NUL terminated name.
    proc_address(unsafe { CStr::from_ptr(name) })
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn finds_the_functions_of_the_system_opengl() {
        for name in [c"glGetString", c"glBindFramebuffer", c"glGenVertexArrays"] {
            assert!(!super::proc_address(name).is_null(), "{name:?}");
        }
        assert!(super::proc_address(c"glNoSuchFunction").is_null());
    }
}
