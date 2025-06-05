use std::sync::Arc;
use std::time::Duration;
use std::sync::atomic::Ordering;
use tokio::time::sleep;
use tokio::fs::{OpenOptions, File};
use tokio::io::{AsyncWriteExt, AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;
use async_channel::{Receiver, Sender};
use reqwest::Client;
use anyhow::{Error, Result};
use crate::anyhow;

use crate::models::{Counters, MAX_RETRIES};
use crate::lookup::{nojs, js, verify_hit};
use crate::utils::create_client;
use crate::workers::ProgressBars;
use crate::auth;
use crate::botguard;
use tracing::error;
use crate::cli::{Mode, LookupType};

/// Worker function that processes phone numbers or emails from the queue
pub async fn worker(
    counters: Arc<Counters>, 
    input_rx: Receiver<String>, 
    output_tx: Sender<String>, 
    subnet: String, 
    first_name: String, 
    last_name: String,
    mode: Mode,
    lookup_type: LookupType
) {
    let mut client: Client = create_client(Some(&subnet), "");
    
    // Track authentication refresh times
    let mut last_auth_refresh = std::time::Instant::now();
    let auth_refresh_interval = Duration::from_secs(8 * 60 * 60); // Refresh auth every 8 hours

    // Set up botguard token for this worker
    botguard::set_bg_firstname(&first_name);
    botguard::set_bg_lastname(&last_name);
    
    // Try to initialize botguard token if not already set
    if botguard::get_bg_token().is_none() {
        if let Err(e) = botguard::force_bg_update().await {
            error!("Initial botguard token setup failed: {}", e);
            // Continue anyway, the lookup function will retry
        }
    }

    'main: while let Ok(identifier) = input_rx.recv().await {
        // Check if we need to refresh authentication
        if last_auth_refresh.elapsed() >= auth_refresh_interval {
            // Try to refresh auth credentials
            if let Ok(_) = auth::get_auth_credentials().await {
                last_auth_refresh = std::time::Instant::now();
            } else {
                // Still continue with old credentials
                error!("Failed to refresh auth credentials");
            }
        }

        if !identifier.contains('@') {
            // Validate phone number
            let parsed_number = format!("+{}", identifier).parse::<phonenumber::PhoneNumber>().unwrap();
            if !phonenumber::is_valid(&parsed_number) {
                counters.success.fetch_add(1, Ordering::Relaxed);
                continue
            }
        }
        
        for attempt in 0..MAX_RETRIES {
            counters.requests.fetch_add(1, Ordering::Relaxed);

            let lookup_result = match lookup_type {
                LookupType::Js => js::lookup(&client, &identifier, &first_name, &last_name).await,
                LookupType::NoJS => nojs::lookup(&client, &identifier, &first_name, &last_name).await,
            };

            match lookup_result {
                Ok(exists) => {
                    counters.success.fetch_add(1, Ordering::Relaxed);

                    if exists {
                        // For emails, we don't need to verify with fake names
                        if mode == Mode::Email {
                            counters.hits.fetch_add(1, Ordering::Relaxed);
                            if let Err(e) = output_tx.send(identifier.clone()).await {
                                error!("Failed to send hit to output channel: {}", e);
                            }
                        } else {
                            // For phone numbers, try verifying with fake names
                            match verify_hit(&client, &identifier, &first_name, &last_name).await {
                                Ok(is_real) => {
                                    if is_real {
                                        counters.hits.fetch_add(1, Ordering::Relaxed);
                                        if let Err(e) = output_tx.send(identifier.clone()).await {
                                            error!("Failed to send hit to output channel: {}", e);
                                        }
                                    }
                                },
                                Err(_) => {
                                    // If verification fails, retry
                                    sleep(Duration::from_millis(100)).await;
                                    continue;
                                }
                            }
                        }
                    }

                    continue 'main;
                }
                Err(error) => {
                    let error_str = error.to_string();
                    
                    if error_str == "ratelimited" {
                        counters.ratelimits.fetch_add(1, Ordering::Relaxed);
                        client = create_client(Some(&subnet), "");
                        // Add a small delay between retries
                        sleep(Duration::from_millis(100)).await;
                        continue;
                    } else if error_str == "invalid_identifier" {
                        counters.success.fetch_add(1, Ordering::Relaxed);
                        continue 'main;
                    } else if error_str.contains("botguard") {
                        // Botguard token issue, try to force an update and retry
                        //if let Err(e) = botguard::force_bg_update().await {
                        //    error!("Failed to update botguard token after error: {}", e);
                        //}
                        error!("Failed to update botguard token after error: {}", error);
                        counters.errors.fetch_add(1, Ordering::Relaxed);
                        sleep(Duration::from_millis(500)).await;
                        continue;
                    } else {
                        error!("unknown error when doing lookup: {}", error);
                        counters.errors.fetch_add(1, Ordering::Relaxed);
                        
                        // If we've tried enough times, move on to the next item
                        if attempt >= MAX_RETRIES - 1 {
                            continue 'main;
                        }
                    }
                }
            }
        }
    }
}


/// Queue work from a file, filtering by prefix, suffix and infix if provided
/// Returns the estimated total number of items to process
pub async fn queue_from_file(
    input_tx: Sender<String>, 
    file_path: &str, 
    prefix: &str, 
    suffix: &str,
    infix: Option<&str>
) -> Result<(), Error> {
    // Check if file exists
    if !tokio::fs::try_exists(file_path).await? {
        return Err(anyhow!("File not found: {}", file_path));
    }
    
    // Check if file is empty
    let metadata = tokio::fs::metadata(file_path).await?;
    if metadata.len() == 0 {
        return Err(anyhow!("File is empty: {}", file_path));
    }
    
    // Process the file 
    let file = File::open(file_path).await?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    
    let mut actual_count = 0;
    let check_suffix = !suffix.is_empty();
    let check_prefix = !prefix.is_empty();
    let check_infix = infix.is_some();
    
    while let Some(line) = lines.next_line().await? {
        // Skip empty lines
        if line.trim().is_empty() {
            continue;
        }
        
        let phone = line.trim();
        
        // Check prefix and suffix conditions
        let suffix_match = !check_suffix || phone.ends_with(suffix);
        let prefix_match = !check_prefix || phone.starts_with(prefix);
        
        // Check infix if needed
        let infix_match = if check_infix {
            let infix_val = infix.unwrap();
            if phone.len() >= 6 {
                // Extract the infix (6th and 5th characters from the end)
                let potential_infix = &phone[phone.len() - 6..phone.len() - 4];
                potential_infix == infix_val
            } else {
                false // Phone number too short for infix
            }
        } else {
            true // No infix check needed
        };
        
        // Only queue if all checks pass
        if suffix_match && prefix_match && infix_match {
            if let Err(error) = input_tx.send(phone.to_string()).await {
                error!("Failed to send to channel: {}", error);
            }
            actual_count += 1;
        }
    }
    
    // If we didn't find any matching numbers, return an error
    if actual_count == 0 {
        return Err(anyhow!("No matching phone numbers found in file: {}", file_path));
    }
    
    Ok(())
}


/// Metrics reporting task that uses progress bars
pub async fn report_metrics(
    counters: Arc<Counters>, 
    input_rx: Receiver<String>, 
    initial_total: u64,
    estimate_rx: Receiver<u64>,
    latest_hit: Arc<Mutex<Option<String>>>
) {
    // Create progress bars with initial estimate
    let progress = ProgressBars::new(initial_total);
    
    // Initialize last values for calculating delta
    let mut last_requests = 0;
    let mut last_time = std::time::Instant::now();
    
    // Use a shorter interval for smoother progress updates
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    
    loop {
        tokio::select! {
            // Check for updated estimate
            Ok(new_estimate) = estimate_rx.recv() => {
                // Update the progress bar with improved estimate
                progress.update_progress(counters.requests.load(Ordering::Relaxed) as u64, Some(new_estimate));
            }
            
            _ = interval.tick() => {
                let requests = counters.requests.load(Ordering::Relaxed);
                let hits = counters.hits.load(Ordering::Relaxed);
                
                // Calculate requests per second based on delta since last update
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(last_time).as_millis() as f64 / 1000.0;
                let req_delta = requests as i64 - last_requests as i64;
                let req_per_sec = if elapsed > 0.0 { (req_delta as f64 / elapsed) as u64 } else { 0 };
                
                // Get latest hit for display
                let hit_str = {
                    let hit_lock = latest_hit.lock().await;
                    hit_lock.clone()
                };
                
                // Update progress bars
                progress.update_progress(requests as u64, None);
                progress.update_stats(&counters, req_per_sec);
                progress.update_hits(hits as u64, hit_str.as_deref());
                
                // Update last values for next calculation
                last_requests = requests;
                last_time = now;
                
                // Stop metrics reporting if all workers have completed
                if input_rx.is_closed() && input_rx.is_empty() {
                    // Make sure all output processing is completed before finishing
                    let current_hits = counters.hits.load(Ordering::Relaxed);
                    
                    // Get latest hit for display in final message
                    let hit_str = {
                        let hit_lock = latest_hit.lock().await;
                        hit_lock.clone()
                    };
                    
                    // Finish the progress bars
                    progress.finish(current_hits as u64, hit_str.as_deref());
                    
                    // Break out of the loop to terminate the task
                    break;
                }
            }
        }
    }
}

/// Handles writing successful hits to the output file
pub async fn output_writer(output_rx: Receiver<String>, latest_hit: Arc<Mutex<Option<String>>>) {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open("output.txt")
        .await.unwrap();

    while let Ok(identifier) = output_rx.recv().await {
        // Update the latest hit for display in the progress bar
        {
            let mut hit = latest_hit.lock().await;
            *hit = Some(identifier.clone());
        }
        
        let line = format!("{}\n", identifier);
        file.write(line.as_bytes()).await.unwrap();
    }
}