use std::ffi::OsStr;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, ResumeThread, TerminateProcess, CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED,
    PROCESS_INFORMATION, STARTUPINFOW,
};

use super::{EnvironmentError, InfraExecutionError, WindowsQemuProcess};

fn win32_error(operation: &str) -> InfraExecutionError {
    let code = unsafe { GetLastError() };
    InfraExecutionError::Environment(EnvironmentError::Io {
        operation: operation.to_string(),
        details: format!("win32_error_code={code}"),
    })
}

pub(super) fn spawn_qemu_windows_raw(
    qemu_boot_cmd: &str,
) -> Result<WindowsQemuProcess, InfraExecutionError> {
    let mut startup_info: STARTUPINFOW = unsafe { mem::zeroed() };
    startup_info.cb = mem::size_of::<STARTUPINFOW>() as u32;
    let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };

    let mut command_line = OsStr::new(qemu_boot_cmd)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();

    let created = unsafe {
        CreateProcessW(
            ptr::null(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_SUSPENDED | CREATE_NEW_PROCESS_GROUP,
            ptr::null(),
            ptr::null(),
            &mut startup_info,
            &mut process_info,
        )
    };

    if created == 0 {
        return Err(win32_error("CreateProcessW"));
    }

    let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if job == 0 {
        unsafe {
            let _ = TerminateProcess(process_info.hProcess, 1);
            CloseHandle(process_info.hThread);
            CloseHandle(process_info.hProcess);
        }
        return Err(win32_error("CreateJobObjectW"));
    }

    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let set_info = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if set_info == 0 {
        unsafe {
            let _ = TerminateProcess(process_info.hProcess, 1);
            CloseHandle(job);
            CloseHandle(process_info.hThread);
            CloseHandle(process_info.hProcess);
        }
        return Err(win32_error("SetInformationJobObject"));
    }

    let assigned = unsafe { AssignProcessToJobObject(job, process_info.hProcess) };
    if assigned == 0 {
        unsafe {
            let _ = TerminateProcess(process_info.hProcess, 1);
            CloseHandle(job);
            CloseHandle(process_info.hThread);
            CloseHandle(process_info.hProcess);
        }
        return Err(win32_error("AssignProcessToJobObject"));
    }

    let resumed = unsafe { ResumeThread(process_info.hThread) };
    if resumed == u32::MAX {
        unsafe {
            let _ = TerminateProcess(process_info.hProcess, 1);
            CloseHandle(job);
            CloseHandle(process_info.hThread);
            CloseHandle(process_info.hProcess);
        }
        return Err(win32_error("ResumeThread"));
    }

    unsafe {
        CloseHandle(process_info.hThread);
    }

    Ok(WindowsQemuProcess {
        process_handle: process_info.hProcess,
        job_handle: job,
        pid: process_info.dwProcessId,
    })
}
