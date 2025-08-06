use helios_sof::{ContentType, SofError};

#[test]
fn test_shortened_format_names() -> Result<(), SofError> {
    // Test shortened format names
    assert_eq!(ContentType::from_string("csv")?, ContentType::CsvWithHeader);
    assert_eq!(ContentType::from_string("json")?, ContentType::Json);
    assert_eq!(ContentType::from_string("ndjson")?, ContentType::NdJson);
    assert_eq!(ContentType::from_string("parquet")?, ContentType::Parquet);

    // Verify full MIME types still work
    assert_eq!(
        ContentType::from_string("text/csv")?,
        ContentType::CsvWithHeader
    );
    assert_eq!(
        ContentType::from_string("text/csv;header=true")?,
        ContentType::CsvWithHeader
    );
    assert_eq!(
        ContentType::from_string("text/csv;header=false")?,
        ContentType::Csv
    );
    assert_eq!(
        ContentType::from_string("application/json")?,
        ContentType::Json
    );
    assert_eq!(
        ContentType::from_string("application/ndjson")?,
        ContentType::NdJson
    );
    assert_eq!(
        ContentType::from_string("application/parquet")?,
        ContentType::Parquet
    );

    // Test invalid format
    assert!(ContentType::from_string("xml").is_err());
    assert!(ContentType::from_string("text/plain").is_err());

    Ok(())
}

#[test]
fn test_csv_header_handling_with_shortened_names() -> Result<(), SofError> {
    // CSV without header info defaults to with header
    let csv = ContentType::from_string("csv")?;
    assert_eq!(csv, ContentType::CsvWithHeader);

    // No shortened version for csv without headers - must use full MIME type
    let csv_no_header = ContentType::from_string("text/csv;header=false")?;
    assert_eq!(csv_no_header, ContentType::Csv);

    Ok(())
}
