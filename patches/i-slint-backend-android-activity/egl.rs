// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::num::NonZeroU32;

type EGLDisplay = *mut c_void;
type EGLConfig = *mut c_void;
type EGLSurface = *mut c_void;
type EGLContext = *mut c_void;
type EGLint = i32;
type EGLBoolean = c_uint;

const EGL_DEFAULT_DISPLAY: *mut c_void = std::ptr::null_mut();
const EGL_NO_CONTEXT: EGLContext = std::ptr::null_mut();
const EGL_NO_SURFACE: EGLSurface = std::ptr::null_mut();
const EGL_NO_DISPLAY: EGLDisplay = std::ptr::null_mut();
const EGL_TRUE: EGLBoolean = 1;

const EGL_SURFACE_TYPE: EGLint = 0x3033;
const EGL_WINDOW_BIT: EGLint = 0x0004;
const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
const EGL_OPENGL_ES2_BIT: EGLint = 0x0004;
const EGL_OPENGL_ES3_BIT: EGLint = 0x00000040;
const EGL_RED_SIZE: EGLint = 0x3024;
const EGL_GREEN_SIZE: EGLint = 0x3023;
const EGL_BLUE_SIZE: EGLint = 0x3022;
const EGL_ALPHA_SIZE: EGLint = 0x3021;
const EGL_STENCIL_SIZE: EGLint = 0x3026;
const EGL_NATIVE_VISUAL_ID: EGLint = 0x302e;
const EGL_NONE: EGLint = 0x3038;
const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;

const RTLD_NOW: c_int = 2;

#[link(name = "EGL")]
#[link(name = "GLESv2")]
#[link(name = "android")]
unsafe extern "C" {
    fn eglGetDisplay(display_id: *mut c_void) -> EGLDisplay;
    fn eglInitialize(dpy: EGLDisplay, major: *mut EGLint, minor: *mut EGLint) -> EGLBoolean;
    fn eglTerminate(dpy: EGLDisplay) -> EGLBoolean;
    fn eglChooseConfig(
        dpy: EGLDisplay,
        attrib_list: *const EGLint,
        configs: *mut EGLConfig,
        config_size: EGLint,
        num_config: *mut EGLint,
    ) -> EGLBoolean;
    fn eglGetConfigAttrib(
        dpy: EGLDisplay,
        config: EGLConfig,
        attribute: EGLint,
        value: *mut EGLint,
    ) -> EGLBoolean;
    fn eglCreateWindowSurface(
        dpy: EGLDisplay,
        config: EGLConfig,
        win: *mut c_void,
        attrib_list: *const EGLint,
    ) -> EGLSurface;
    fn eglCreateContext(
        dpy: EGLDisplay,
        config: EGLConfig,
        share_context: EGLContext,
        attrib_list: *const EGLint,
    ) -> EGLContext;
    fn eglDestroySurface(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
    fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> EGLBoolean;
    fn eglMakeCurrent(
        dpy: EGLDisplay,
        draw: EGLSurface,
        read: EGLSurface,
        ctx: EGLContext,
    ) -> EGLBoolean;
    fn eglSwapBuffers(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
    fn eglGetProcAddress(procname: *const c_char) -> *const c_void;
    fn eglGetError() -> EGLint;

    fn ANativeWindow_setBuffersGeometry(
        window: *mut c_void,
        width: i32,
        height: i32,
        format: i32,
    ) -> i32;

    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

pub struct AndroidEglContext {
    display: EGLDisplay,
    surface: EGLSurface,
    context: EGLContext,
    gles_lib: *mut c_void,
}

impl AndroidEglContext {
    pub fn new(native_window_ptr: *mut c_void) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if native_window_ptr.is_null() {
            return Err("ANativeWindow pointer is null".into());
        }

        unsafe {
            let display = eglGetDisplay(EGL_DEFAULT_DISPLAY);
            if display == EGL_NO_DISPLAY {
                return Err(format!("eglGetDisplay failed: {:#x}", eglGetError()).into());
            }

            let mut major = 0;
            let mut minor = 0;
            if eglInitialize(display, &mut major, &mut minor) != EGL_TRUE {
                return Err(format!("eglInitialize failed: {:#x}", eglGetError()).into());
            }

            // Try ES3 first, then fallback to ES2. FemtoVG requires an 8-bit stencil buffer.
            let attribs_es3 = [
                EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
                EGL_RENDERABLE_TYPE, EGL_OPENGL_ES3_BIT,
                EGL_RED_SIZE, 8,
                EGL_GREEN_SIZE, 8,
                EGL_BLUE_SIZE, 8,
                EGL_ALPHA_SIZE, 8,
                EGL_STENCIL_SIZE, 8,
                EGL_NONE,
            ];

            let attribs_es2 = [
                EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
                EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
                EGL_RED_SIZE, 8,
                EGL_GREEN_SIZE, 8,
                EGL_BLUE_SIZE, 8,
                EGL_ALPHA_SIZE, 8,
                EGL_STENCIL_SIZE, 8,
                EGL_NONE,
            ];

            let mut config = std::ptr::null_mut();
            let mut num_config = 0;
            let mut gles_version = 3;

            if eglChooseConfig(display, attribs_es3.as_ptr(), &mut config, 1, &mut num_config) != EGL_TRUE
                || num_config < 1
            {
                gles_version = 2;
                if eglChooseConfig(display, attribs_es2.as_ptr(), &mut config, 1, &mut num_config) != EGL_TRUE
                    || num_config < 1
                {
                    return Err(format!("eglChooseConfig failed: {:#x}", eglGetError()).into());
                }
            }

            let mut format = 0;
            if eglGetConfigAttrib(display, config, EGL_NATIVE_VISUAL_ID, &mut format) == EGL_TRUE {
                ANativeWindow_setBuffersGeometry(native_window_ptr, 0, 0, format);
            }

            let surface = eglCreateWindowSurface(display, config, native_window_ptr, std::ptr::null());
            if surface == EGL_NO_SURFACE {
                return Err(format!("eglCreateWindowSurface failed: {:#x}", eglGetError()).into());
            }

            let context_attribs = [
                EGL_CONTEXT_CLIENT_VERSION, gles_version,
                EGL_NONE,
            ];

            let context = eglCreateContext(display, config, EGL_NO_CONTEXT, context_attribs.as_ptr());
            if context == EGL_NO_CONTEXT {
                eglDestroySurface(display, surface);
                return Err(format!("eglCreateContext failed: {:#x}", eglGetError()).into());
            }

            if eglMakeCurrent(display, surface, surface, context) != EGL_TRUE {
                eglDestroyContext(display, context);
                eglDestroySurface(display, surface);
                return Err(format!("eglMakeCurrent failed: {:#x}", eglGetError()).into());
            }

            // Open libGLESv2.so (or libGLESv3.so) for core symbol resolution fallback
            let gles_lib = dlopen(b"libGLESv3.so\0".as_ptr().cast(), RTLD_NOW);
            let gles_lib = if gles_lib.is_null() {
                dlopen(b"libGLESv2.so\0".as_ptr().cast(), RTLD_NOW)
            } else {
                gles_lib
            };

            Ok(Self {
                display,
                surface,
                context,
                gles_lib,
            })
        }
    }
}

impl Drop for AndroidEglContext {
    fn drop(&mut self) {
        unsafe {
            eglMakeCurrent(self.display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
            if self.surface != EGL_NO_SURFACE {
                eglDestroySurface(self.display, self.surface);
            }
            if self.context != EGL_NO_CONTEXT {
                eglDestroyContext(self.display, self.context);
            }
            if self.display != EGL_NO_DISPLAY {
                eglTerminate(self.display);
            }
            if !self.gles_lib.is_null() {
                dlclose(self.gles_lib);
            }
        }
    }
}

unsafe impl i_slint_renderer_femtovg::opengl::OpenGLInterface for AndroidEglContext {
    fn ensure_current(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        unsafe {
            if eglMakeCurrent(self.display, self.surface, self.surface, self.context) != EGL_TRUE {
                return Err(format!("eglMakeCurrent failed: {:#x}", eglGetError()).into());
            }
        }
        Ok(())
    }

    fn swap_buffers(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        unsafe {
            if eglSwapBuffers(self.display, self.surface) != EGL_TRUE {
                return Err(format!("eglSwapBuffers failed: {:#x}", eglGetError()).into());
            }
        }
        Ok(())
    }

    fn resize(&self, _width: NonZeroU32, _height: NonZeroU32) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Android ANativeWindow automatically adapts the surface size.
        Ok(())
    }

    fn get_proc_address(&self, name: &CStr) -> *const c_void {
        unsafe {
            let proc = eglGetProcAddress(name.as_ptr());
            if !proc.is_null() {
                return proc;
            }
            if !self.gles_lib.is_null() {
                let proc = dlsym(self.gles_lib, name.as_ptr());
                if !proc.is_null() {
                    return proc;
                }
            }
            // Fallback to global symbol table (RTLD_DEFAULT is NULL on Android Bionic)
            dlsym(std::ptr::null_mut(), name.as_ptr())
        }
    }
}
