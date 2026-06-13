//! ZIP archive helper for multi-file Parquet output.
//!
//! When a Parquet export is split into multiple files (via
//! `parquet_options.max_file_size_mb`), the files are bundled into a single ZIP
//! archive for delivery. The archive is built fully in memory and returned as a
//! sized response body — there is no chunked transfer encoding, because the
//! total size is known before the response starts (see
//! `crates/sof/docs/spec-inconsistencies.md`, entry F).

use std::io::{Cursor, Write};

use zip::{CompressionMethod, ZipWriter, write::FileOptions};

/// Build a ZIP archive from multiple Parquet file buffers.
///
/// Files are named `{base_name}.parquet`, `{base_name}_002.parquet`, … . The
/// `Stored` (no-compression) method is used because Parquet data is already
/// compressed.
pub fn create_zip_from_buffers(
    file_buffers: Vec<Vec<u8>>,
    base_name: &str,
) -> Result<Vec<u8>, crate::error::ServerError> {
    let mut zip_buffer = Vec::new();
    let cursor = Cursor::new(&mut zip_buffer);
    let mut zip = ZipWriter::new(cursor);

    let options = FileOptions::<()>::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(true);

    for (i, buffer) in file_buffers.iter().enumerate() {
        let file_name = if i == 0 {
            format!("{}.parquet", base_name)
        } else {
            format!("{}_{:03}.parquet", base_name, i + 1)
        };

        zip.start_file(file_name, options).map_err(|e| {
            crate::error::ServerError::InternalError(format!(
                "Failed to start ZIP file entry: {}",
                e
            ))
        })?;

        zip.write_all(buffer).map_err(|e| {
            crate::error::ServerError::InternalError(format!("Failed to write to ZIP: {}", e))
        })?;
    }

    zip.finish().map_err(|e| {
        crate::error::ServerError::InternalError(format!("Failed to finish ZIP: {}", e))
    })?;

    Ok(zip_buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_zip_from_buffers() {
        let buffers = vec![vec![1u8, 2, 3, 4], vec![5u8, 6, 7, 8]];

        let zip_data = create_zip_from_buffers(buffers, "test").unwrap();

        // ZIP file should have proper header
        assert!(!zip_data.is_empty());
        // ZIP files start with "PK"
        assert_eq!(&zip_data[0..2], b"PK");
    }
}
