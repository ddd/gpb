use std::path::Path;
use tokio::fs::File;
use anyhow::{Error, Result, anyhow};
use tokio::io::{AsyncWriteExt, BufWriter};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

// Structure to represent a CSV record with serde support
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CsvRecord {
    pub identifier: String,
    pub masked_number: String,
    pub first_name: String,
    pub last_name: String,
}

// Structure to represent a processed hit
#[derive(Clone, Debug, Serialize)]
pub struct CsvHit {
    pub identifier: String,
    pub phone: String,
    pub first_name: String,
    pub last_name: String,
}

// Parse CSV input file containing masked numbers using csv library
pub async fn parse_csv_input(file_path: &str) -> Result<Vec<CsvRecord>, Error> {
    // Check if file exists
    if !Path::new(file_path).exists() {
        return Err(anyhow!("CSV file not found: {}", file_path));
    }
    
    // Open and read the file
    let file_content = tokio::fs::read_to_string(file_path).await?;
    
    // Use the csv crate to parse the file
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .flexible(true)
        .from_reader(Cursor::new(file_content));
    
    // Parse records
    let mut records = Vec::new();
    
    // Deserialize each record
    for (idx, result) in reader.deserialize::<CsvRecord>().enumerate() {
        match result {
            Ok(record) => {
                records.push(record);
            },
            Err(e) => {
                return Err(anyhow!("Error parsing CSV record at line {}: {}", idx + 2, e));
            }
        }
    }
    
    if records.is_empty() {
        return Err(anyhow!("No valid records found in CSV file"));
    }
    
    Ok(records)
}

// Initialize CSV output file with header
pub async fn initialize_csv_output(file_path: &str) -> Result<(), Error> {
    let file = File::create(file_path).await?;
    let mut writer = BufWriter::new(file);
    
    // Create a CSV writer
    let mut csv_writer = csv::WriterBuilder::new()
        .from_writer(vec![]);
    
    // Write header using the CsvHit struct field names
    csv_writer.write_record(&["identifier", "phone", "firstname", "lastname"])?;
    
    // Get the CSV content as bytes
    let csv_content = csv_writer.into_inner()?;
    
    // Write to file
    writer.write_all(&csv_content).await?;
    writer.flush().await?;
    
    Ok(())
}

// Append a hit to the CSV output file
pub async fn append_csv_hit(file_path: &str, hit: &CsvHit) -> Result<(), Error> {
    // Read the existing file to avoid header rewriting issues
    let existing_content = match tokio::fs::try_exists(file_path).await {
        Ok(true) => tokio::fs::read_to_string(file_path).await?,
        _ => String::new(),
    };
    
    // Open file for writing
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(file_path)
        .await?;
    
    let mut writer = BufWriter::new(file);
    
    // Create a CSV writer for the new record
    let mut csv_writer = csv::WriterBuilder::new()
        .from_writer(vec![]);
    
    // Serialize the hit
    csv_writer.serialize(hit)?;
    
    // Get the CSV content as bytes
    let mut csv_content = csv_writer.into_inner()?;
    
    // If there's existing content, we need to handle appending properly
    if !existing_content.is_empty() {
        // Write existing content first
        writer.write_all(existing_content.as_bytes()).await?;
        
        // For the new content, skip the header line
        let new_content = String::from_utf8(csv_content)?;
        let lines: Vec<&str> = new_content.lines().collect();
        
        // Only take the data line (skip header)
        if lines.len() > 1 {
            csv_content = lines[1].as_bytes().to_vec();
            writer.write_all(&csv_content).await?;
            writer.write_all(b"\n").await?;
        }
    } else {
        // No existing content, write everything including header
        writer.write_all(&csv_content).await?;
    }
    
    writer.flush().await?;
    
    Ok(())
}