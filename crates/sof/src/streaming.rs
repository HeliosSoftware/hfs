//! Streaming utilities for handling large Parquet files and chunked transfer encoding
//!
//! This module provides utilities for streaming large datasets, particularly
//! when generating multiple Parquet files that need to be sent as a single response.

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures::stream;
use std::io::{Cursor, Write};
use tokio_stream::Stream;
use zip::{CompressionMethod, ZipWriter, write::FileOptions};

/// Create a streaming ZIP response containing multiple Parquet files
///
/// This function takes multiple Parquet file buffers and streams them as a ZIP archive
/// using chunked transfer encoding. Each file is added to the ZIP with a sequential name.
///
/// # Arguments
/// * `file_buffers` - Vector of Parquet file data buffers
/// * `base_name` - Base name for files in the ZIP (e.g., "data" produces "data.parquet", "data_002.parquet", etc.)
///
/// # Returns
/// An Axum Response with the ZIP content streamed using chunked transfer encoding
pub fn stream_parquet_zip_response(
    file_buffers: Vec<Vec<u8>>,
    base_name: &str,
) -> Result<Response, crate::error::ServerError> {
    // Create the ZIP archive in memory
    let zip_buffer = create_zip_from_buffers(file_buffers, base_name)?;

    // Create a stream that yields the ZIP data in chunks
    let stream = create_chunked_stream(zip_buffer);

    // Build the response with appropriate headers
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "application/zip".parse().unwrap());
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{}.zip\"", base_name)
            .parse()
            .unwrap(),
    );
    // Transfer-Encoding: chunked is automatically set by Axum when using streaming body

    let body = Body::from_stream(stream);

    Ok((StatusCode::OK, headers, body).into_response())
}

/// Create a ZIP archive from multiple file buffers
fn create_zip_from_buffers(
    file_buffers: Vec<Vec<u8>>,
    base_name: &str,
) -> Result<Vec<u8>, crate::error::ServerError> {
    let mut zip_buffer = Vec::new();
    let cursor = Cursor::new(&mut zip_buffer);
    let mut zip = ZipWriter::new(cursor);

    // Use STORE method (no compression) since Parquet files are already compressed
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

/// Create a chunked stream from a buffer
///
/// This splits the buffer into chunks suitable for streaming over HTTP
fn create_chunked_stream(
    buffer: Vec<u8>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    // Use 64KB chunks for efficient streaming
    const CHUNK_SIZE: usize = 65536;

    let chunks: Vec<Bytes> = buffer
        .chunks(CHUNK_SIZE)
        .map(Bytes::copy_from_slice)
        .collect();

    stream::iter(chunks.into_iter().map(Ok))
}

/// Stream a single large Parquet file with chunked transfer encoding
///
/// This function takes a single Parquet file buffer and streams it using chunked transfer encoding.
///
/// # Arguments
/// * `parquet_data` - The Parquet file data
///
/// # Returns
/// An Axum Response with the Parquet content streamed using chunked transfer encoding
pub fn stream_single_parquet_response(
    parquet_data: Vec<u8>,
) -> Result<Response, crate::error::ServerError> {
    // Create a stream that yields the data in chunks
    let stream = create_chunked_stream(parquet_data);

    // Build the response with appropriate headers
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "application/parquet".parse().unwrap());
    headers.insert(
        header::CONTENT_DISPOSITION,
        "attachment; filename=\"data.parquet\"".parse().unwrap(),
    );

    let body = Body::from_stream(stream);

    Ok((StatusCode::OK, headers, body).into_response())
}

/// Calculate the total size of all buffers
#[allow(dead_code)]
pub fn calculate_total_size(buffers: &[Vec<u8>]) -> usize {
    buffers.iter().map(|b| b.len()).sum()
}

/// Check if streaming is beneficial based on data size
///
/// Returns true if the data is large enough that streaming provides benefits
pub fn should_use_streaming(total_size: usize) -> bool {
    // Stream if data is larger than 10MB
    const STREAMING_THRESHOLD: usize = 10 * 1024 * 1024;
    total_size > STREAMING_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_total_size() {
        let buffers = vec![vec![0u8; 1000], vec![0u8; 2000], vec![0u8; 3000]];
        assert_eq!(calculate_total_size(&buffers), 6000);
    }

    #[test]
    fn test_should_use_streaming() {
        // Small data shouldn't use streaming
        assert!(!should_use_streaming(1024));

        // Large data should use streaming
        assert!(should_use_streaming(20 * 1024 * 1024));
    }

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
