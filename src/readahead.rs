// SPDX-FileCopyrightText: 2025 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::collections::HashMap;
use std::ffi::CStr;
use std::fs::{metadata, read_dir, write, File, OpenOptions};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
#[cfg(feature = "readahead-debug")]
use std::process;
use std::process::exit;

use log::{debug, error, warn};
use memmap::Mmap;
use miniserde::de::{self, Visitor};
use miniserde::ser::{self, Fragment};
use miniserde::{json, make_place, Deserialize, Serialize};
use nix::errno::Errno;
use nix::fcntl::{posix_fadvise, splice, PosixFadviseAdvice::POSIX_FADV_WILLNEED, SpliceFFlags};
use nix::libc::{off_t, O_NONBLOCK, O_TMPFILE};
use nix::poll::{ppoll, PollFd, PollFlags};
use nix::sys::prctl::set_name;
use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::SignalFd;
use nix::sys::stat::{fstat, major, minor};
use nix::unistd::{fork, pipe, ForkResult};

use crate::util::Result;
use crate::{mount::mount_apivfs, util::read_file};

#[cfg(target_pointer_width = "64")]
const SIZE_OF_LONG: usize = 8;

#[cfg(target_pointer_width = "32")]
const SIZE_OF_LONG: usize = 4;

const KBUFFER_TYPE_PADDING: u32 = 29;
const KBUFFER_TYPE_TIME_EXTEND: u32 = 30;
const KBUFFER_TYPE_TIME_STAMP: u32 = 31;

struct TraceFile {
    trace: File,
    pipe_read: OwnedFd,
    pipe_write: OwnedFd,
    tmp: File,
    cpu: usize,
}

struct TraceData {
    mmap: Mmap,
    pages: usize,
    timestamp: u64,
    page: usize,
    page_size: usize,
    page_offset: usize,
    record_end: usize,
    record_type: u16,
}

make_place!(Place);

struct ReadaheadBlock {
    file: Option<usize>,
    offset: u64,
    size: u64,
    ino: u64,
    #[cfg(feature = "readahead-debug")]
    pid: u32,
}

struct ReadaheadBlockStream<'a> {
    block: &'a ReadaheadBlock,
    file: u64,
    pos: usize,
}

struct ReadaheadBlockBuilder<'a> {
    out: &'a mut Option<ReadaheadBlock>,
    file: Option<usize>,
    offset: u64,
    size: u64,
    element: Option<u64>,
    pos: usize,
}

#[derive(Serialize, Deserialize)]
pub struct ReadaheadData {
    files: Vec<String>,
    blocks: Vec<ReadaheadBlock>,
}

impl<'a> ser::Seq for ReadaheadBlockStream<'a> {
    fn next(&mut self) -> Option<&dyn Serialize> {
        let pos = self.pos;
        self.pos += 1;
        match pos {
            0 => Some(&self.file),
            1 => Some(&self.block.offset),
            2 => Some(&self.block.size),
            _ => None,
        }
    }
}

impl Serialize for ReadaheadBlock {
    fn begin(&self) -> Fragment<'_> {
        Fragment::Seq(Box::new(ReadaheadBlockStream {
            block: self,
            file: self.file.unwrap() as u64,
            pos: 0,
        }))
    }
}

impl<'a> ReadaheadBlockBuilder<'a> {
    fn take(&mut self) {
        if let Some(next) = self.element.take() {
            let pos = self.pos;
            match pos {
                0 => self.file = Some(next as usize),
                1 => self.offset = next,
                2 => self.size = next,
                _ => (),
            }
            self.pos += 1;
        }
    }
}

impl<'a> de::Seq for ReadaheadBlockBuilder<'a> {
    fn element(&mut self) -> miniserde::Result<&mut dyn Visitor> {
        self.take();
        Ok(Deserialize::begin(&mut self.element))
    }
    fn finish(&mut self) -> miniserde::Result<()> {
        self.take();
        *self.out = Some(ReadaheadBlock {
            file: self.file,
            offset: self.offset,
            size: self.size,
            ino: 0,
            #[cfg(feature = "readahead-debug")]
            pid: 0,
        });
        Ok(())
    }
}

impl Visitor for Place<ReadaheadBlock> {
    fn seq(&mut self) -> miniserde::Result<Box<dyn miniserde::de::Seq + '_>> {
        Ok(Box::new(ReadaheadBlockBuilder {
            out: &mut self.out,
            file: None,
            offset: 0,
            size: 0,
            element: None,
            pos: 0,
        }))
    }
}

impl Deserialize for ReadaheadBlock {
    fn begin(out: &mut Option<Self>) -> &mut dyn Visitor {
        Place::new(out)
    }
}

fn write_file(filename: &str, data: &str) -> std::result::Result<(), String> {
    write(filename, data).map_err(|e| format!("Failed to write {filename}: {e}"))
}

pub fn enable_readahead_tracing(enable: bool) -> Result<()> {
    let value = if enable { "1" } else { "0" };
    write_file(
        "/sys/kernel/tracing/events/filemap/mm_filemap_add_to_page_cache/enable",
        value,
    )?;
    write_file("/sys/kernel/tracing/tracing_on", value)?;
    Ok(())
}

fn open_trace(cpu: u8) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(O_NONBLOCK)
        .open(format!(
            "/sys/kernel/tracing/per_cpu/cpu{cpu}/trace_pipe_raw"
        ))?)
}

fn open_traces() -> Result<Vec<TraceFile>> {
    let mut i: u8 = 0;
    let mut traces: Vec<TraceFile> = Vec::new();

    while let Ok(trace_file) = open_trace(i) {
        let (pipe_read, pipe_write) = pipe().unwrap();
        traces.push(TraceFile {
            trace: trace_file,
            tmp: OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(O_NONBLOCK | O_TMPFILE)
                .open("/run")
                .map_err(|e| format!("Failed to create temporary file in /run: {e}"))?,
            cpu: i as usize,
            pipe_read,
            pipe_write,
        });
        i += 1;
    }
    Ok(traces)
}

fn do_splice<Fd1: std::os::fd::AsFd, Fd2: std::os::fd::AsFd>(
    src: Fd1,
    dst: Fd2,
    flags: SpliceFFlags,
    cpu: usize,
) -> Result<bool> {
    match splice(src, None, dst, None, 4096, flags) {
        Ok(count) => {
            if count < 4096 {
                return Err(format!(
                    "unexpected short ({count}) short trace buffer read for cpu{cpu}"
                )
                .into());
            }
        }
        Err(Errno::EAGAIN) => {
            return Ok(false);
        }
        Err(err) => {
            return Err(format!("Unexpected error reading trace buffer for cpu{cpu}: {err}").into())
        }
    }
    Ok(true)
}

fn read_trace(file: &TraceFile) -> Result<()> {
    let mut flags = SpliceFFlags::empty();
    flags.insert(SpliceFFlags::SPLICE_F_NONBLOCK);
    flags.insert(SpliceFFlags::SPLICE_F_MOVE);
    loop {
        if !do_splice(&file.trace, &file.pipe_write, flags, file.cpu)? {
            break;
        }
        do_splice(&file.pipe_read, &file.tmp, flags, file.cpu)?;
    }
    Ok(())
}

macro_rules! trace_pop {
    ($T:ty, $data:expr) => {{
        let start = $data.page * 4096 + $data.page_offset;
        let size = size_of::<$T>();
        $data.page_offset += size;
        <$T>::from_ne_bytes($data.mmap[start..start + size].try_into().unwrap())
    }};
}

macro_rules! trace_read {
    ($T:ty, $data:expr, $offset:expr) => {{
        let start = $data.page * 4096 + $data.page_offset + $offset;
        let size = size_of::<$T>();
        <$T>::from_ne_bytes($data.mmap[start..start + size].try_into().unwrap())
    }};
}

fn split_type_len(type_len_ts: u32) -> (u32, u64) {
    (type_len_ts & ((1 << 5) - 1), (type_len_ts >> 5) as u64)
}

fn trace_next(data: &mut TraceData) {
    data.page_offset = data.record_end;
    loop {
        if data.page_offset >= data.page_size {
            if data.page + 1 == data.pages {
                // end of the trace reached
                data.timestamp = u64::MAX;
                return;
            }
            data.page += 1;
            start_trace_page(data);
        }
        let type_len_ts = trace_pop!(u32, data);
        let (type_len, delta) = split_type_len(type_len_ts);
        match type_len {
            KBUFFER_TYPE_PADDING => {
                // padding size includes type_len_ts
                let padding = trace_pop!(u32, data) as usize - 4;
                assert!(data.page_offset + padding <= data.page_size);
                data.page_offset += padding;
                data.timestamp += delta;
            }
            KBUFFER_TYPE_TIME_EXTEND => {
                let extend = trace_pop!(u32, data) as u64;
                data.timestamp += (extend << 17) + delta;
            }
            KBUFFER_TYPE_TIME_STAMP => {
                let extend = trace_pop!(u32, data) as u64;
                data.timestamp = (data.timestamp & (0xf8 << 56)) + (extend << 27) + delta;
            }
            _ => {
                data.record_end = data.page_offset
                    + if type_len == 0 {
                        ((trace_pop!(u32, data) as usize - 4) + 3) & !3
                    } else {
                        type_len as usize * 4
                    };
                data.timestamp += delta;
                data.record_type = trace_read!(u16, data, 0);
                break;
            }
        }
    }
    assert!(data.page_offset < data.page_size);
}

fn start_trace_page(data: &mut TraceData) {
    data.page_offset = 0;
    data.timestamp = trace_pop!(u64, data);
    let flags: u64 = if SIZE_OF_LONG == 8 {
        trace_pop!(u64, data)
    } else {
        trace_pop!(u32, data) as u64
    };
    data.page_size = (flags & ((1 << 27) - 1)) as usize + data.page_offset;
    data.record_end = data.page_offset;
}

fn setup_trace_data(file: &TraceFile) -> Option<TraceData> {
    if fstat(&file.tmp).unwrap().st_size == 0 {
        return None;
    }
    let mmap = unsafe { Mmap::map(&file.tmp) }.unwrap();
    let pages = mmap.len() / 4096;
    let mut data = TraceData {
        mmap,
        pages,
        timestamp: 0,
        page: 0,
        page_size: 0,
        page_offset: 0,
        record_end: 0,
        record_type: 0,
    };
    start_trace_page(&mut data);
    trace_next(&mut data);
    Some(data)
}

fn find_paths(
    dir: &Path,
    root_dev: u64,
    data: &mut Vec<ReadaheadBlock>,
    map: &HashMap<u64, Vec<usize>>,
    files: &mut Vec<String>,
) -> Result<()> {
    for entry in read_dir(dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.dev() != root_dev {
            continue;
        }
        let path = entry.path();
        if metadata.is_dir() {
            find_paths(&path, root_dev, data, map, files)?;
        } else if let Some(list) = map.get(&metadata.ino()) {
            let index = files.len();
            files.push(path.to_str().unwrap().to_string());
            for i in list {
                data[*i].file = Some(index);
            }
        }
    }
    Ok(())
}

fn readahead_start_trace() -> Result<()> {
    mount_apivfs("/sys/kernel/tracing", "tracefs")?;
    enable_readahead_tracing(true)?;
    Ok(())
}

pub fn readahead_trace(_verify: bool) -> Result<ReadaheadData> {
    let traces = open_traces()?;
    let mut fds: Vec<PollFd> = traces
        .iter()
        .map(|trace| PollFd::new(trace.trace.as_fd(), PollFlags::POLLIN))
        .collect();
    let mut mask = SigSet::empty();
    mask.add(Signal::SIGHUP);
    mask.add(Signal::SIGINT);
    mask.add(Signal::SIGTERM);
    mask.thread_block().unwrap();
    let sigfd = SignalFd::new(&mask).unwrap();
    fds.push(PollFd::new(sigfd.as_fd(), PollFlags::POLLIN));
    'main: loop {
        let mut ready = ppoll(&mut fds, None, Some(mask))?;
        if ready == 0 {
            break;
        }
        // check trace fds
        for (i, fd) in fds.split_last().unwrap().1.iter().enumerate() {
            if fd.any().unwrap_or_default() {
                read_trace(&traces[i])?;
                ready -= 1;
            }
            if ready == 0 {
                break;
            }
        }
        if ready != 0 {
            // check signal fd
            if fds.last().unwrap().any().unwrap_or_default() {
                break 'main;
            } else {
                break;
            }
        }
    }
    debug!("Stop readahead tracing...");
    enable_readahead_tracing(false)?;

    let format =
        read_file("/sys/kernel/tracing/events/filemap/mm_filemap_add_to_page_cache/format")?;
    let format: Vec<&str> = format.split("\n").collect();
    // The second line contains "ID: <id>"
    assert!(format[1][..4] == *"ID: ");
    let id = format[1][4..].to_string().parse::<u16>().unwrap();

    for trace in traces.iter() {
        read_trace(trace)?;
    }
    let mut data: Vec<TraceData> = traces.iter().filter_map(setup_trace_data).collect();
    if data.is_empty() {
        return Err("No trace data!".into());
    }
    let root_metadata = metadata("/").unwrap();
    let root_dev = root_metadata.dev();
    let root_major = major(root_dev);
    let root_minor = minor(root_dev);

    let large = SIZE_OF_LONG == 8;
    let mut readahead_data: Vec<ReadaheadBlock> = Vec::new();
    let mut readahead_map: HashMap<u64, Vec<usize>> = HashMap::new();
    loop {
        let mut next: usize = 0;
        for (i, trace_data) in data.iter().enumerate().skip(1) {
            if trace_data.timestamp < data[next].timestamp {
                next = i;
            }
        }
        let trace_data = &mut data[next];
        if trace_data.timestamp == u64::MAX {
            break;
        }
        if trace_data.record_type != id {
            warn!(
                "Skipping unexpected record type {} for cpu{next}",
                trace_data.record_type
            );
            trace_next(trace_data);
            continue;
        }
        let s_dev = trace_read!(u32, trace_data, if large { 32 } else { 20 });
        let minor = (s_dev & ((1 << 8) - 1)) as u64;
        let major = (s_dev >> 20) as u64;
        if major == root_major && minor == root_minor {
            let ino = if large {
                trace_read!(u64, trace_data, 16)
            } else {
                trace_read!(u32, trace_data, 12) as u64
            };
            let mut offset = if large {
                trace_read!(u64, trace_data, 24)
            } else {
                trace_read!(u32, trace_data, 16) as u64
            };
            let mut size = 1 << trace_read!(u8, trace_data, if large { 36 } else { 24 });
            #[cfg(feature = "readahead-debug")]
            let pid = trace_read!(u32, trace_data, 4);
            if let Some(last) = readahead_data.last() {
                // merge blocks if possible
                if last.ino == ino && last.offset + last.size == offset {
                    offset = last.offset;
                    size += last.size;
                    readahead_data.pop();
                }
            }
            readahead_data.push(ReadaheadBlock {
                file: None,
                offset,
                size,
                ino,
                #[cfg(feature = "readahead-debug")]
                pid,
            });
            let index = readahead_data.len() - 1;
            readahead_map
                .entry(ino)
                .and_modify(|list| list.push(index))
                .or_insert(Vec::from([index]));
        }
        trace_next(trace_data);
    }
    let mut files = Vec::new();
    find_paths(
        Path::new("/"),
        root_dev,
        &mut readahead_data,
        &readahead_map,
        &mut files,
    )?;
    #[cfg(feature = "readahead-debug")]
    if _verify {
        let pid = process::id();
        for block in readahead_data.iter() {
            if block.pid != pid {
                debug!(
                    "pid: {:8} offset {:3} blocks: {:3} {}",
                    block.pid,
                    block.offset,
                    block.size,
                    if let Some(file) = block.file {
                        files[file].to_string()
                    } else {
                        block.ino.to_string()
                    }
                );
            }
        }
    }
    readahead_data.retain(|block| block.file.is_some());
    let readahead = ReadaheadData {
        files,
        blocks: readahead_data,
    };
    Ok(readahead)
}

pub fn readahead_write(readahead: &ReadaheadData, filename: &str) -> Result<()> {
    debug!("Writing readahead file {filename}.");
    write_file(filename, &json::to_string(&readahead))?;
    Ok(())
}

fn try_open(filename: &str) -> Option<OwnedFd> {
    match File::open(filename) {
        Ok(file) => Some(OwnedFd::from(file)),
        Err(err) => {
            debug!("Failed to open {filename}: {err}");
            None
        }
    }
}

pub fn readahead_load(readahead: &ReadaheadData) {
    let mut fds: HashMap<usize, Option<OwnedFd>> = HashMap::new();
    for block in readahead.blocks.iter() {
        let index = block.file.unwrap();
        if let Some(fd) = fds
            .entry(index)
            .or_insert_with(|| try_open(&readahead.files[index]))
        {
            posix_fadvise(
                fd,
                (block.offset * 4096) as off_t,
                (block.size * 4096) as off_t,
                POSIX_FADV_WILLNEED,
            )
            .unwrap();
        }
    }
    fds.clear();
    debug!("Readahead done.");
}

fn readahead_run(trace_file: &str, input: Option<ReadaheadData>) -> Result<()> {
    if let Some(input) = input {
        readahead_load(&input);
        let _ = readahead_trace(true)?;
    } else {
        let readahead = readahead_trace(false)?;
        readahead_write(&readahead, trace_file)?;
    }
    Ok(())
}

pub fn readahead_open(trace_file: &str) -> Result<Option<ReadaheadData>> {
    if Path::new(trace_file).exists() {
        debug!("Loading readahead file {trace_file}...");
        let data = read_file(trace_file)?;
        let readahead: ReadaheadData = json::from_str(&data)?;
        Ok(Some(readahead))
    } else {
        debug!("Readahead file {trace_file} not found. Start tracing...");
        Ok(None)
    }
}

pub fn readahead_start(input: Option<ReadaheadData>, trace_file: &str) -> Result<()> {
    if let Err(err) = readahead_start_trace() {
        error!("Start tracing failed with: {err}");
        if input.is_none() {
            return Ok(());
        }
    }
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child, .. }) => {
            debug!("Started readahead as pid {child}");
            return Ok(());
        }
        Ok(ForkResult::Child) => {
            // for rust 1.75
            #[allow(clippy::manual_c_str_literals)]
            let _ = set_name(CStr::from_bytes_with_nul(b"readahead\0").unwrap());
            if let Err(err) = readahead_run(trace_file, input) {
                error!("Readahead failed with{err}");
            }
            exit(0);
        }
        Err(err) => error!("Fork readahead process failed: {err}!"),
    }
    Ok(())
}
