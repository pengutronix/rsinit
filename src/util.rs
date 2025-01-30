use std::{
    fs::{create_dir_all, read_to_string, write},
    path::Path,
};

use crate::Result;

pub(crate) fn mkdir(dir: impl AsRef<Path>) -> Result<()> {
    create_dir_all(dir.as_ref()).map_err(|e| {
        format!(
            "Failed to create directory {}: {e}",
            dir.as_ref().to_string_lossy()
        )
        .into()
    })
}

pub(crate) fn read_file(filename: impl AsRef<Path>) -> Result<String> {
    read_to_string(filename.as_ref()).map_err(|e| {
        format!(
            "Failed to read {}: {e}",
            filename.as_ref().to_string_lossy()
        )
        .into()
    })
}

pub(crate) fn write_file<C: AsRef<[u8]>>(path: impl AsRef<Path>, content: C) -> Result<()> {
    write(&path, content).map_err(|e| {
        format!(
            "Failed to write to {}: {e}",
            path.as_ref().to_string_lossy()
        )
        .into()
    })
}
