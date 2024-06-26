use crate::arch::{escape_to_string, parse_args};
use crate::style::StyleConfig;
use libc::{c_ulonglong, user_regs_struct};
use nix::unistd::Pid;
use serde::ser::{SerializeMap, SerializeSeq};
use serde::Serialize;
use serde::__private::ser::FlatMapSerializer;
use serde_json::Value;
use std::borrow::Cow;
use std::fmt::{Debug, Display};
use std::io;
use std::io::Write;
use std::time::Duration;
use syscalls::Sysno;

#[derive(Debug)]
pub struct SyscallInfo {
    pub typ: &'static str,
    pub pid: Pid,
    pub syscall: Sysno,
    pub args: SyscallArgs,
    pub result: RetCode,
    pub duration: Duration,
}

impl SyscallInfo {
    pub fn new(
        pid: Pid,
        syscall: Sysno,
        ret_code: RetCode,
        registers: user_regs_struct,
        duration: Duration,
    ) -> Self {
        Self {
            typ: "SYSCALL",
            pid,
            syscall,
            args: parse_args(pid, syscall, registers),
            result: ret_code,
            duration,
        }
    }

    pub fn write_syscall(
        &self,
        style: StyleConfig,
        string_limit: Option<usize>,
        show_syscall_num: bool,
        show_duration: bool,
        output: &mut dyn Write,
    ) -> anyhow::Result<()> {
        if style.use_colors {
            write!(output, "[{}] ", style.pid.apply_to(&self.pid.to_string()))?;
        } else {
            write!(output, "[{}] ", &self.pid)?;
        }
        if show_syscall_num {
            write!(output, "{:>3} ", self.syscall.id())?;
        }
        if style.use_colors {
            let styled = style.syscall.apply_to(self.syscall.to_string());
            write!(output, "{styled}(")
        } else {
            write!(output, "{}(", &self.syscall)
        }?;
        for (idx, arg) in self.args.0.iter().enumerate() {
            if idx > 0 {
                write!(output, ", ")?;
            }
            arg.write(output, string_limit)?;
        }
        write!(output, ") = ")?;
        if self.syscall == Sysno::exit || self.syscall == Sysno::exit_group {
            write!(output, "?")?;
        } else {
            if style.use_colors {
                let style = style.from_ret_code(self.result);
                // TODO: it would be great if we can force termcolor to write
                //       the styling prefix and suffix into the formatter.
                //       This would allow us to use the same code for both cases,
                //       and avoid additional string alloc
                write!(output, "{}", style.apply_to(self.result.to_string()))
            } else {
                write!(output, "{}", self.result)
            }?;
            if show_duration {
                // TODO: add an option to control each syscall duration scaling, e.g. ms, us, ns
                write!(output, " <{:.6}ns>", self.duration.as_nanos())?;
            }
        }
        Ok(writeln!(output)?)
    }
}

impl Serialize for SyscallInfo {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(7))?;
        map.serialize_entry("type", &self.typ)?;
        map.serialize_entry("pid", &self.pid.as_raw())?;
        map.serialize_entry("num", &self.syscall)?;
        map.serialize_entry("syscall", &self.syscall.to_string())?;
        map.serialize_entry("args", &self.args)?;
        Serialize::serialize(&self.result, FlatMapSerializer(&mut map))?;
        map.serialize_entry("duration", &self.duration.as_secs_f64())?;
        map.end()
    }
}

#[derive(Debug)]
pub struct SyscallArgs(pub Vec<SyscallArg>);

impl Serialize for SyscallArgs {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for arg in &self.0 {
            let value = match arg {
                SyscallArg::Int(v) => serde_json::to_value(v).unwrap(),
                SyscallArg::Bytes(v) => serde_json::to_value(escape_to_string(v)).unwrap(),
                SyscallArg::Addr(v) => Value::String(format!("{v:#x}")),
            };
            seq.serialize_element(&value)?;
        }
        seq.end()
    }
}

#[derive(Debug, Copy, Clone, Serialize)]
pub enum RetCode {
    #[serde(rename = "success")]
    Ok(i32),
    #[serde(rename = "error")]
    Err(i32),
    #[serde(rename = "result")]
    Address(usize),
}

impl RetCode {
    pub fn from_raw(ret_code: c_ulonglong) -> Self {
        let ret_i32 = ret_code as isize;
        // TODO: is this > or >= ?  Add a link to the docs.
        if ret_i32.abs() > 0x8000 {
            Self::Address(ret_code as usize)
        } else if ret_i32 < 0 {
            Self::Err(ret_i32 as i32)
        } else {
            Self::Ok(ret_i32 as i32)
        }
    }
}

impl Display for RetCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ok(v) | Self::Err(v) => Display::fmt(v, f),
            Self::Address(v) => write!(f, "{v:#X}"),
        }
    }
}

#[derive(Debug, Serialize)]
pub enum SyscallArg {
    Int(i64),
    Bytes(Vec<u8>),
    Addr(usize),
}

impl SyscallArg {
    pub fn write(&self, f: &mut dyn Write, string_limit: Option<usize>) -> io::Result<()> {
        match self {
            Self::Int(v) => write!(f, "{v}"),
            Self::Bytes(v) => {
                let mut buf: Vec<u8> = Cow::Borrowed(v).into_owned();
                if let Some(width) = string_limit {
                    if buf.len() > width {
                        buf.truncate(width);
                        let _ = buf.write(b"...");
                    }
                }
                write!(f, "{}", serde_json::to_value(escape_to_string(&buf))?)?;
                Ok(())
            }
            Self::Addr(v) => write!(f, "{v:#X}"),
        }
    }
}
