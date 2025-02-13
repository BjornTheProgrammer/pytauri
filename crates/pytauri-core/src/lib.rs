pub mod ipc;
pub mod tauri_runtime;
pub mod utils;

use std::error::Error;
use std::fmt::{Debug, Display};
use std::ops::Deref;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::{IntoPyObject, PyErr};
use pyo3_utils::{PyMatchRef, PyWrapper, PyWrapperT0, PyWrapperT2, UnsafeUngilExt};
use tauri::Manager;

use crate::tauri_runtime::Runtime;

#[pyclass(frozen)]
#[non_exhaustive]
pub enum RunEventEnum {
    Exit(),
    #[non_exhaustive]
    ExitRequested {
        code: Option<i32>,
        // TODO, XXX, FIXME: `ExitRequestApi` is a private type in `tauri`,
        // we need create a issue to `tauri`, or we cant implement this.
        // api: ExitRequestApi,
    },
    #[non_exhaustive]
    WindowEvent {
        label: String,
        // TODO:
        // event: WindowEvent,
    },
    #[non_exhaustive]
    WebviewEvent {
        label: String,
        // TODO:
        // event: WebviewEvent,
    },
    Ready(),
    Resumed(),
    MainEventsCleared(),
    MenuEvent(/* TODO: tauri::menu::MenuEvent */),
    // TODO:
    // TrayIconEvent(tauri::tray::TrayIconEvent),
}

#[pyclass(frozen)]
#[non_exhaustive]
pub struct RunEvent(pub PyWrapper<PyWrapperT0<tauri::RunEvent>>);

impl PyMatchRef for RunEvent {
    type Output = RunEventEnum;

    fn match_ref(&self) -> Self::Output {
        match self.0.inner_ref().deref() {
            tauri::RunEvent::Exit => RunEventEnum::Exit(),
            tauri::RunEvent::ExitRequested {
                code, /* TODO */ ..
            } => RunEventEnum::ExitRequested { code: *code },
            tauri::RunEvent::WindowEvent {
                label, /* TODO */ ..
            } => RunEventEnum::WindowEvent {
                label: label.to_owned(),
            },
            tauri::RunEvent::WebviewEvent {
                label, /* TODO */ ..
            } => RunEventEnum::WebviewEvent {
                label: label.to_owned(),
            },
            tauri::RunEvent::Ready => RunEventEnum::Ready(),
            tauri::RunEvent::Resumed => RunEventEnum::Resumed(),
            tauri::RunEvent::MainEventsCleared => RunEventEnum::MainEventsCleared(),
            tauri::RunEvent::MenuEvent(/* TODO */ _) => RunEventEnum::MenuEvent(),
            // TODO: tauri::RunEvent::TrayIconEvent,
            event => unimplemented!("Unimplemented RunEvent: {event:?}"),
        }
    }
}

#[pymethods]
impl RunEvent {
    fn match_ref(&self) -> <Self as PyMatchRef>::Output {
        <Self as PyMatchRef>::match_ref(self)
    }
}

impl RunEvent {
    const fn new(run_event: tauri::RunEvent) -> Self {
        Self(PyWrapper::new0(run_event))
    }
}

/// Get the global app singleton [Py]<[AppHandle]> through [PyAppHandleExt]
#[pyclass(frozen)]
#[non_exhaustive]
// NOTE: Do not use [PyWrapperT2], otherwise the global singleton [PyAppHandle]
// will be consumed and cannot be used;
// If you really need ownership of [tauri::AppHandle], you can use [tauri::AppHandle::clone].
pub struct AppHandle(pub PyWrapper<PyWrapperT0<tauri::AppHandle<Runtime>>>);

impl AppHandle {
    fn new(app_handle: tauri::AppHandle<Runtime>) -> Self {
        Self(PyWrapper::new0(app_handle))
    }
}

struct PyAppHandle(Py<AppHandle>);

impl PyAppHandle {
    fn new(py_app_handle: Py<AppHandle>) -> Self {
        Self(py_app_handle)
    }
}

impl Deref for PyAppHandle {
    type Target = Py<AppHandle>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
pub struct PyAppHandleStateError;

impl Display for PyAppHandleStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Failed to get `PyAppHandle` from state, maybe this app was not created by pytauri"
        )
    }
}

impl Error for PyAppHandleStateError {}

impl From<PyAppHandleStateError> for PyErr {
    fn from(value: PyAppHandleStateError) -> Self {
        PyRuntimeError::new_err(format!("{value}"))
    }
}

pub type PyAppHandleStateResult<T> = Result<T, PyAppHandleStateError>;

pub trait PyAppHandleExt<R: tauri::Runtime>: Manager<R> {
    /// # Panics
    ///
    /// Panics if it fails to get [PyAppHandle] from the state,
    /// indicating that this app has not initialized by pytauri.
    fn py_app_handle(&self) -> impl Deref<Target = Py<AppHandle>> {
        self.try_py_app_handle().unwrap()
    }

    fn try_py_app_handle(&self) -> PyAppHandleStateResult<impl Deref<Target = Py<AppHandle>>> {
        self.try_state::<PyAppHandle>()
            .map(|state| state.inner().deref())
            .ok_or(PyAppHandleStateError)
    }
}

impl<R: tauri::Runtime, T: Manager<R>> PyAppHandleExt<R> for T {}

#[pyclass(frozen, unsendable)]
#[non_exhaustive]
pub struct App(pub PyWrapper<PyWrapperT2<tauri::App<Runtime>>>);

impl App {
    pub fn try_build(py: Python<'_>, app: tauri::App<Runtime>) -> PyResult<Self> {
        let app_handle = AppHandle::new(app.handle().to_owned());
        let py_app_handle = PyAppHandle::new(app_handle.into_pyobject(py)?.unbind());
        // if false, there has already state set for the app instance.
        if !app.manage(py_app_handle) {
            unreachable!(
                "`PyAppHandle` is private, so it is impossible for other crates to manage it"
            )
        }
        Ok(Self(PyWrapper::new2(app)))
    }

    fn py_cb_to_rs_cb(
        callback: PyObject,
    ) -> impl FnMut(&tauri::AppHandle<Runtime>, tauri::RunEvent) {
        move |app_handle, run_event| {
            let py_app_handle = app_handle.py_app_handle();
            let py_run_event = RunEvent::new(run_event);

            Python::with_gil(|py| {
                let callback = callback.bind(py);
                let result = callback.call1((py_app_handle.clone_ref(py), py_run_event));
                if let Err(e) = result {
                    // Use [write_unraisable] instead of [restore]:
                    // - Because we are about to panic, Python might abort
                    // - [restore] will not be handled in this case, so it will not be printed to stderr
                    e.write_unraisable(py, Some(callback));
                    // `panic` allows Python to exit `app.run()`,
                    // otherwise the Python main thread will be blocked by `app.run()`
                    // and unable to raise an error
                    panic!("Python exception occurred in callback")
                }
            })
        }
    }

    fn noop_callback(_: &tauri::AppHandle<Runtime>, _: tauri::RunEvent) {}
}

#[pymethods]
impl App {
    #[pyo3(signature = (callback = None, /))]
    fn run(&self, py: Python<'_>, callback: Option<PyObject>) -> PyResult<()> {
        // `self: &App` does not hold the GIL, so this is safe
        unsafe {
            py.allow_threads_unsend(self, |slf| {
                let app = slf.0.try_take_inner()??;
                match callback {
                    Some(callback) => app.run(Self::py_cb_to_rs_cb(callback)),
                    None => app.run(Self::noop_callback),
                }
                Ok(())
            })
        }
    }

    #[pyo3(signature = (callback = None, /))]
    fn run_iteration(&self, py: Python<'_>, callback: Option<PyObject>) -> PyResult<()> {
        unsafe {
            // `self: &App` does not hold the GIL, so this is safe
            py.allow_threads_unsend(self, |slf| {
                let mut app = slf.0.try_lock_inner_mut()??;
                match callback {
                    Some(callback) => app.run_iteration(Self::py_cb_to_rs_cb(callback)),
                    None => app.run_iteration(Self::noop_callback),
                }
                Ok(())
            })
        }
    }

    fn cleanup_before_exit(&self, py: Python<'_>) -> PyResult<()> {
        // `self: &App` does not hold the GIL, so this is safe
        unsafe {
            py.allow_threads_unsend(self, |slf| {
                let app = slf.0.try_lock_inner_ref()??;
                app.cleanup_before_exit();
                Ok(())
            })
        }
    }
}

#[pyclass(frozen)]
#[non_exhaustive]
pub struct Context(pub PyWrapper<PyWrapperT2<tauri::Context>>);

impl Context {
    pub fn new(context: tauri::Context) -> Self {
        Self(PyWrapper::new2(context))
    }
}

#[pymodule(submodule, gil_used = false, name = "pytauri")]
pub mod ext_mod {
    use super::*;

    #[pymodule_export]
    pub use crate::{App, AppHandle, Context, RunEvent, RunEventEnum};

    #[pymodule]
    pub mod ipc {
        #[pymodule_export]
        pub use crate::ipc::{Invoke, InvokeResolver};
    }
}
