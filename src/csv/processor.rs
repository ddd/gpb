use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::time::interval;
use anyhow::{Error, Result, anyhow};
use tracing::error;

use crate::cli::{Args, LookupType};
use crate::models::Counters;
use crate::format::{get_country_format, PhoneNumberGenerator};
use crate::workers::ProgressBars;
use crate::csv::parser::{CsvHit, parse_csv_input, initialize_csv_output, append_csv_hit};
use crate::csv::worker::{WorkerMessage, ResultMessage, csv_worker};
use crate::botguard;
use std::sync::atomic::AtomicUsize;

// Process CSV mode with persistent worker pool
pub async fn process_csv_mode(args: &Args) -> Result<(), Error> {
    // Parse the CSV file to get all records
    let csv_records = match parse_csv_input(args.input_file.as_ref().unwrap()).await {
        Ok(records) => records,
        Err(e) => return Err(anyhow!("Failed to parse CSV file: {}", e))
    };
    
    let total_records = csv_records.len();
    println!("Loaded {} records from CSV file", total_records);
    
    // Initialize output CSV file
    let output_file = "output.csv";
    if let Err(e) = initialize_csv_output(output_file).await {
        return Err(anyhow!("Failed to initialize output CSV file: {}", e));
    }
    
    // Set up channels for worker communication
    let (work_tx, work_rx) = async_channel::bounded::<WorkerMessage>(1000);
    let (result_tx, result_rx) = async_channel::bounded::<ResultMessage>(1000);

    // Create shared counters
    let counters = Arc::new(Counters::new());
    
    // Create progress bars
    let progress = ProgressBars::new(100); // Initial length will be updated
    progress.update_message(&format!("Processing CSV with {} records", total_records));
    
    // Create latest hit tracking
    let latest_hit = Arc::new(tokio::sync::Mutex::new(None::<String>));

    // Start the worker pool - these will run for the entire duration
    let mut worker_handles = vec![];
    for _ in 0..args.workers {
        let worker_work_rx = work_rx.clone();
        let worker_result_tx = result_tx.clone();
        let worker_counters = Arc::clone(&counters);
        let worker_subnet = args.subnet.clone();
        let worker_lookup_type = args.lookup_type;
        
        let handle = tokio::spawn(async move {
            csv_worker(
                worker_work_rx,
                worker_result_tx,
                worker_counters,
                worker_subnet,
                worker_lookup_type,
            ).await;
        });
        
        worker_handles.push(handle);
    }
    
    // Track found records
    let mut found_records = 0;
    // Track total hits across all records
    let mut total_hits = 0;

    for attempt in 0..3 {
        match botguard::force_bg_update().await {
            Ok(_) => {
                break;
            },
            Err(e) => {
                error!("Failed to update botguard token for record (attempt {}/3): {}", 
                             attempt + 1, e);
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }
    
    // Process each record sequentially
    for (idx, record) in csv_records.iter().enumerate() {
        let record_id = idx;
        
        // Extract info from masked number using the consolidated function
        let phone_info = match crate::utils::extract_info_from_masked_phone(&record.masked_number, None) {
            Ok(info) => info,
            Err(e) => {
                error!("Error processing record {}: {}", idx + 1, e);
                
                // Output NOT_FOUND record for error cases too
                let csv_hit = CsvHit {
                    identifier: record.identifier.clone(),
                    phone: "NOT_FOUND".to_string(),
                    first_name: record.first_name.clone(),
                    last_name: record.last_name.clone(),
                };
                
                if let Err(e) = append_csv_hit(output_file, &csv_hit).await {
                    error!("Error writing to output CSV: {}", e);
                }
                
                continue;
            }
        };

        // Get country format
        let format = match get_country_format(&phone_info.country_code) {
            Ok(format) => format,
            Err(e) => {
                error!("Error getting format for record {}: {}", idx + 1, e);
                
                // Output NOT_FOUND record for error cases too
                let csv_hit = CsvHit {
                    identifier: record.identifier.clone(),
                    phone: "NOT_FOUND".to_string(),
                    first_name: record.first_name.clone(),
                    last_name: record.last_name.clone(),
                };
                
                if let Err(e) = append_csv_hit(output_file, &csv_hit).await {
                    error!("Error writing to output CSV: {}", e);
                }
                
                continue;
            }
        };
        
        // Create number generator with all extracted information
        let mut generator = match PhoneNumberGenerator::new(
            &format,
            phone_info.prefix,           // Use prefix from extracted info
            Some(phone_info.suffix),     // Use suffix from extracted info
            phone_info.infix,            // Use infix from extracted info
            None                         // No digit override
        ) {
            Ok(gen) => gen,
            Err(e) => {
                error!("Error creating generator for record {}: {}", idx + 1, e);
                
                // Output NOT_FOUND record for error cases too
                let csv_hit = CsvHit {
                    identifier: record.identifier.clone(),
                    phone: "NOT_FOUND".to_string(),
                    first_name: record.first_name.clone(),
                    last_name: record.last_name.clone(),
                };
                
                if let Err(e) = append_csv_hit(output_file, &csv_hit).await {
                    error!("Error writing to output CSV: {}", e);
                }
                
                continue;
            }
        };
        
        // Update progress display
        let record_msg = format!("Record {}/{}: ID={}, {} ({})", 
                                 idx + 1, total_records,
                                 record.identifier, 
                                 record.masked_number,
                                 format.code);
        progress.update_message(&record_msg);
        
        // Reset request counters for this record but NOT the hits counter
        counters.requests.store(0, Ordering::Relaxed);
        counters.success.store(0, Ordering::Relaxed);
        counters.errors.store(0, Ordering::Relaxed);
        counters.ratelimits.store(0, Ordering::Relaxed);
        
        // Update progress bar for this number
        let estimated_total = generator.estimate_total();
        progress.reset_position();
        progress.set_length(estimated_total);
        
        // Initialize botguard token with correct name for this record if we're in JS mode
        if args.lookup_type == LookupType::Js {
            botguard::set_bg_firstname(&record.first_name);
            botguard::set_bg_lastname(&record.last_name);
            
            // Retry token update up to 3 times with delay
            let mut token_updated = false;
            for attempt in 0..3 {
                match botguard::force_bg_update().await {
                    Ok(_) => {
                        token_updated = true;
                        break;
                    },
                    Err(e) => {
                        error!("Failed to update botguard token for record {} (attempt {}/3): {}", 
                                idx + 1, attempt + 1, e);
                        if attempt < 2 {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                }
            }
            if !token_updated {
                error!("Warning: Could not update botguard token for record {}. Continuing anyway.", idx + 1);
            }
        }
        
        // Flag to indicate if we should stop generating numbers
        let stop_processing = Arc::new(AtomicBool::new(false));
        
        // Flag to track if all generation is done
        let generation_complete = Arc::new(AtomicBool::new(false));
        
        // Create a channel to collect hits for this record
        let (record_hits_tx, record_hits_rx) = async_channel::bounded::<String>(100);
        
        // Create thread to monitor and collect results for this record
        let record_monitor_handle = {
            let result_rx = result_rx.clone();
            let record_hits_tx = record_hits_tx.clone();
            let stop_processing = Arc::clone(&stop_processing);
            let skip_after_hit = args.skip_after_hit;
            let latest_hit = Arc::clone(&latest_hit);
            let counters = Arc::clone(&counters);
            
            tokio::spawn(async move {
                while let Ok(result) = result_rx.recv().await {
                    match result {
                        ResultMessage::Hit { record_id: rid, phone } => {
                            if rid == record_id {
                                // Update latest hit for display
                                {
                                    let mut hit = latest_hit.lock().await;
                                    *hit = Some(phone.clone());
                                }
                                
                                // Increment hits counter atomically
                                counters.hits.fetch_add(1, Ordering::Relaxed);
                                
                                // Send to record-specific channel
                                if let Err(e) = record_hits_tx.send(phone).await {
                                    error!("Failed to send hit to record channel: {}", e);
                                }
                                
                                // Stop if skip_after_hit is enabled
                                if skip_after_hit {
                                    stop_processing.store(true, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                }
            })
        };
        
        // Create thread to update progress
        let progress_handle = {
            let progress = progress.clone();
            let counters = Arc::clone(&counters);
            let latest_hit = Arc::clone(&latest_hit);
            let stop_flag = Arc::clone(&stop_processing);
            
            tokio::spawn(async move {
                let mut last_requests = 0;
                let mut last_time = Instant::now();
                let mut update_interval = interval(Duration::from_millis(200));
                
                while !stop_flag.load(Ordering::Relaxed) {
                    update_interval.tick().await;
                    
                    let requests = counters.requests.load(Ordering::Relaxed);
                    let hits = counters.hits.load(Ordering::Relaxed);
                    
                    // Calculate requests per second
                    let now = Instant::now();
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
                }
            })
        };
        
        // Create a sync counter to track pending tasks
        let pending_tasks = Arc::new(AtomicUsize::new(0));
        
        // Task to enqueue phone numbers for workers
        let enqueue_handle = {
            let work_tx = work_tx.clone();
            let stop_processing = Arc::clone(&stop_processing);
            let generation_complete = Arc::clone(&generation_complete);
            let first_name = record.first_name.clone();
            let last_name = record.last_name.clone();
            let identifier = record.identifier.clone();
            let pending_tasks = Arc::clone(&pending_tasks);
            
            tokio::spawn(async move {
                while let Some(phone) = generator.next() {
                    // Check if we should stop
                    if stop_processing.load(Ordering::Relaxed) {
                        break;
                    }
                    
                    // Increment pending tasks counter
                    pending_tasks.fetch_add(1, Ordering::SeqCst);
                    
                    // Send to workers
                    if let Err(e) = work_tx.send(WorkerMessage::CheckPhone {
                        record_id,
                        phone,
                        identifier: identifier.clone(),
                        first_name: first_name.clone(),
                        last_name: last_name.clone(),
                        pending_counter: Some(Arc::clone(&pending_tasks)),
                    }).await {
                        // Decrement counter since this task won't be processed
                        pending_tasks.fetch_sub(1, Ordering::SeqCst);
                        error!("Failed to send phone to workers: {}", e);
                        break;
                    }
                }
                
                // Signal that generation is complete for this record
                generation_complete.store(true, Ordering::Relaxed);
            })
        };
        
        // Collect all hits for this record
        let mut record_hits = Vec::new();

        // Keep track of time for overall timeout
        let wait_start_time = Instant::now();
        let max_wait_time = Duration::from_secs(300); // 5 minutes maximum wait time (increased)
        
        // Variables for stall detection - only used after generation is complete
        let mut last_success_count = 0;
        let mut last_req_count = 0;
        let mut last_activity_time = Instant::now();
        let stall_detection_timeout = Duration::from_secs(45); // 45 seconds of no activity (increased)
        
        // Keep collecting hits
        loop {
            // Check all conditions
            let current_pending = pending_tasks.load(Ordering::SeqCst);
            let is_generation_complete = generation_complete.load(Ordering::Relaxed);
            let is_stopped = stop_processing.load(Ordering::Relaxed);
            let total_wait_time = wait_start_time.elapsed();
            
            // Get current activity counters
            let current_success = counters.success.load(Ordering::Relaxed);
            let current_requests = counters.requests.load(Ordering::Relaxed);
            
            // Check for activity by monitoring success and request counts
            let has_activity = current_success != last_success_count || 
                              current_requests != last_req_count;
            
            if has_activity {
                // Activity detected, reset the timer
                last_activity_time = Instant::now();
                last_success_count = current_success;
                last_req_count = current_requests;
            }
            
            // Only check for stalls if generation is complete
            // This prevents premature termination during active generation
            let stalled = is_generation_complete && 
                         current_pending > 0 && 
                         last_activity_time.elapsed() > stall_detection_timeout;
            
            // Exit conditions
            let tasks_complete = current_pending == 0 && is_generation_complete;
            let timed_out = total_wait_time > max_wait_time;
            
            // Log when we detect important conditions
            if stalled && is_generation_complete {
                error!("⚠️ Stall detected - no activity for {} seconds with {} pending tasks. Generation complete: {}",
                         last_activity_time.elapsed().as_secs(), current_pending, is_generation_complete);
            }
            
            if is_stopped || tasks_complete || (timed_out && is_generation_complete) || (stalled && is_generation_complete) {
                // Only terminate due to stall or timeout if generation is actually complete
                if stalled && !is_stopped && !tasks_complete && is_generation_complete {
                    error!("⚠️ Terminating search for record {} due to stalled workers. {} tasks still pending after {} seconds of inactivity.",
                             idx + 1, current_pending, last_activity_time.elapsed().as_secs());
                } else if timed_out && !is_stopped && !tasks_complete && is_generation_complete {
                    error!("⚠️ Terminating search for record {} due to timeout. {} tasks still pending after {} seconds total time.",
                             idx + 1, current_pending, total_wait_time.as_secs());
                }
                
                break;
            }
            
            // Try to collect results with a short timeout
            match tokio::time::timeout(Duration::from_millis(100), record_hits_rx.recv()).await {
                Ok(Ok(hit)) => {
                    // Got a hit
                    record_hits.push(hit);
                    
                    // Check if we should stop after first hit
                    if args.skip_after_hit {
                        stop_processing.store(true, Ordering::Relaxed);
                        break;
                    }
                },
                Ok(Err(_)) => {
                    // Channel closed or empty
                    break;
                },
                Err(_) => {
                    // Timeout, continue waiting
                    continue;
                }
            }
        }
        
        // Check for any remaining hits in the channel without blocking too long
        // This ensures we don't miss hits that came in right at the end
        if !record_hits.is_empty() || pending_tasks.load(Ordering::SeqCst) > 0 {
            for _ in 0..5 {  // Try up to 5 times with very short timeouts
                match tokio::time::timeout(Duration::from_millis(5), record_hits_rx.recv()).await {
                    Ok(Ok(hit)) => {
                        record_hits.push(hit);
                        // If we got a hit, try a few more times
                        continue;
                    },
                    _ => break
                }
            }
        }
        
        // Ensure stop flag is set to stop progress thread
        stop_processing.store(true, Ordering::Relaxed);
        
        // Stop the progress thread
        progress_handle.abort();
        
        // Wait for enqueuing to finish
        if let Err(e) = enqueue_handle.await {
            error!("Error in number generation: {:?}", e);
        }
        
        // Cancel the monitor task - we'll create a new one for the next record
        record_monitor_handle.abort();
        
        // Process hits for this record
        let csv_hit = CsvHit {
            identifier: record.identifier.clone(),
            phone: if record_hits.is_empty() {
                // No hits found, use "NOT_FOUND"
                "NOT_FOUND".to_string()
            } else {
                found_records += 1;
                total_hits += record_hits.len();
                
                if args.skip_after_hit || record_hits.len() == 1 {
                    // Single hit mode
                    record_hits[0].clone()
                } else {
                    // Multiple hits - join with colon
                    record_hits.join(":")
                }
            },
            first_name: record.first_name.clone(),
            last_name: record.last_name.clone(),
        };
        
        // Write to output file
        if let Err(e) = append_csv_hit(output_file, &csv_hit).await {
            error!("Error writing to output CSV: {}", e);
        } else {
            if record_hits.is_empty() {
                println!("❌ No hits found for: ID={}, {}", 
                         record.identifier, record.masked_number);
            } else {
                println!("✅ Found: ID={}, {} -> {}", 
                         record.identifier, record.masked_number, csv_hit.phone);
            }
        }
        
        // Add a small delay between records to ensure clean transition
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    // Signal all workers to shut down
    for _ in 0..args.workers {
        work_tx.send(WorkerMessage::Shutdown).await.ok();
    }
    
    // Wait for all workers to finish
    for (i, handle) in worker_handles.into_iter().enumerate() {
        if let Err(e) = handle.await {
            error!("Worker {} shutdown error: {:?}", i, e);
        }
    }
    
    // Finish progress display
    progress.csv_finish(total_records, found_records);
    
    println!("CSV processing complete. Results saved to {}", output_file);
    println!("Total hits: {}, Records with at least one hit: {}", total_hits, found_records);
    Ok(())
}