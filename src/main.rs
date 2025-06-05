mod cli;
mod models;
mod lookup;
mod workers;
mod utils;
mod auth;
mod format;
mod csv;
mod botguard;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::path::Path;
use anyhow::{Error, Result, anyhow};
use clap::Parser;
use tokio::sync::Mutex as TokioMutex;

use crate::cli::{Args, Mode};
use crate::models::Counters;
use crate::workers::{worker, output_writer, queue_from_file, report_metrics};
use crate::utils::{calculate_estimate, verify_subnet_for_country, load_mask_data};
use crate::format::{get_country_format, PhoneNumberGenerator, load_format_data};
use crate::csv::process_csv_mode;

#[tokio::main]
async fn main() -> Result<(), Error> {
    if let Err(e) = utils::check_ulimit() {
        eprintln!("ERROR: {}", e);
        eprintln!("The program cannot run with this ulimit. To fix:");
        eprintln!("  1. Run 'ulimit -n 1000000' in your terminal");
        eprintln!("  2. Then restart this program");
        // Exit the program as the ulimit is too low
        return Err(anyhow!("Insufficient ulimit for operation"));
    }

    let file_appender = tracing_appender::rolling::hourly("logs", "gpb.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .json()
        .init();
    
    // Parse command line arguments
    let args = Args::parse();

    // Check if botguard server is running or static token is provided
    if !crate::botguard::ping_botguard_server().await {
        if args.botguard_token.is_none() {
            return Err(anyhow!(
                "Botguard token generation server not found at http://localhost:7912. \
                Either start the server or provide a static token with the -b option."
            ));
        }
    }
    
    // Try to load format data
    if let Err(e) = load_format_data() {
        eprintln!("Warning: Could not load format data: {}", e);
        eprintln!("Some features may not work correctly without format data.");
    }
    
    // Try to load mask data if a masked phone is provided
    if args.masked_phone.is_some() {
        if let Err(e) = load_mask_data() {
            return Err(anyhow!("Could not load mask data: {}", e));
        }
    }
    
    // Validate arguments
    if args.mode == Mode::Quick && args.country_code.is_none() && args.input_file.is_none() {
        return Err(anyhow!("Either country code (-c) or input file (-i) is required when using quick mode"));
    }
        
    // First name and last name validation for non-Blacklist and non-CSV modes
    if args.mode != Mode::Blacklist && args.mode != Mode::Csv {
        if args.first_name.is_empty() {
            return Err(anyhow!("First name (-f) is required for {:?} mode", args.mode));
        }
    }
        
    // Email mode requires an input file
    if (args.mode == Mode::Email || args.mode == Mode::Csv) && args.input_file.is_none() {
        return  Err(anyhow!("Input file (-i) is required for {:?} mode", args.mode));
    }

    
    // Pre-fetch authentication credentials before starting the workers
    if let Err(e) = auth::get_auth_credentials().await {
        return Err(anyhow!("Failed to fetch authentication credentials: {}", e));
    }

    if let Some(token) = &args.botguard_token {
        // If a static botguard token is provided, use it instead of fetching a new one
        botguard::set_bg_firstname(&args.first_name);
        botguard::set_bg_lastname(&args.last_name);
        botguard::set_static_bg_token(token);
    } else if args.mode != Mode::Blacklist && args.mode != Mode::Csv {
        // Otherwise, use the regular token refresh mechanism
        botguard::set_bg_firstname(&args.first_name);
        botguard::set_bg_lastname(&args.last_name);
        
        if let Err(e) = botguard::force_bg_update().await {
            return Err(anyhow!("Failed to initialize botguard token: {}", e));
        }
    }
    
    // Start the background token refresh task
    botguard::start_bg_token_refresh_task().await;
    
    // Special handling for CSV mode
    if args.mode == Mode::Csv {
        return process_csv_mode(&args).await;
    }
    
    // Process masked phone if provided (works with any mode)
    let mut derived_country_code = None;
    let mut derived_suffix = None;
    let mut derived_prefix = None;
    let mut derived_infix = None;

    if let Some(masked_phone) = args.masked_phone.as_ref() {
        // Use our new consolidated extraction function
        match utils::extract_info_from_masked_phone(masked_phone, args.country_code.as_deref()) {
            Ok(info) => {
                println!("Detected country code: {} from mask pattern", info.country_code);
                println!("Extracted suffix: {}", info.suffix);
                
                // Store the extracted information for later use
                derived_country_code = Some(info.country_code);
                derived_suffix = Some(info.suffix);
                
                // Display and store prefix if present
                if let Some(prefix) = &info.prefix {
                    println!("Extracted prefix: {}", prefix);
                    derived_prefix = Some(prefix.clone());
                }
                
                // Display and store infix if present
                if let Some(infix) = &info.infix {
                    println!("Extracted infix: {}", infix);
                    derived_infix = Some(infix.clone());
                }
            },
            Err(e) => {
                return Err(anyhow!("Failed to process masked phone: {}. If multiple countries match, please specify a country code with -c.", e));
            }
        }
    }
    
    // Determine the suffix to use (from args or derived from mask)
    let effective_suffix = if let Some(derived) = derived_suffix {
        Some(derived)
    } else {
        args.suffix.clone()
    };
    
    // Determine the prefix to use (from args or derived from mask)
    let effective_prefix = if let Some(derived) = derived_prefix {
        Some(derived)
    } else {
        args.prefix.clone()
    };

    // Determine the country code to use (from args or derived from mask)
    let effective_country_code = if let Some(derived) = derived_country_code {
        Some(derived)
    } else {
        args.country_code.clone()
    };
    
    // Determine the infix to use (from args or derived from mask)
    let effective_infix = if let Some(derived) = derived_infix {
        Some(derived)
    } else {
        None
        //args.infix.clone()
    };

    if args.mode == Mode::Blacklist {
        if let Err(e) = botguard::force_bg_update().await {
            return Err(anyhow!("Failed to initialize botguard token: {}", e));
        }
        // For blacklist mode, country code is now optional
        if let Some(country_code) = &effective_country_code {
            // Check for specific country
            match crate::utils::verify_subnet_for_country(&args.subnet, country_code, 3).await {
                Ok(_) => {
                    println!("✅ Subnet {} is NOT blacklisted for country: {}", args.subnet, country_code);
                    return Ok(());
                },
                Err(e) => {
                    println!("❌ Error: {}", e);
                    return Err(e);
                }
            }
        } else {
            // No country specified, check all countries
            println!("No country code specified. Checking all countries with blacklist data...");
            
            match crate::utils::check_all_countries_blacklist(&args.subnet).await {
                Ok(blacklisted) => {
                    if blacklisted.is_empty() {
                        println!("✅ Subnet {} is not blacklisted for any checked country.", args.subnet);
                    } else {
                        println!("❌ Subnet {} is blacklisted for the following countries:", args.subnet);
                        for country in &blacklisted {
                            println!("  - {}", country);
                        }
                    }
                    return Ok(());
                },
                Err(e) => {
                    println!("❌ Error checking countries: {}", e);
                    return Err(e);
                }
            }
        }
    }
    
    // Determine the input file path for quick mode
    let input_file_path = if args.mode == Mode::Quick {
        if let Some(country_code) = &effective_country_code {
            let file_path = format!("data/lbg/{}.lst", country_code.to_lowercase());
            
            // Check if the country file exists
            if !Path::new(&file_path).exists() {
                return Err(anyhow!("No data found for country code: {}. File not found: {}", country_code, file_path));
            }
            
            Some(file_path)
        } else {
            args.input_file.clone()
        }
    } else if args.mode == Mode::Email {
        // For email mode, use the specified input file
        args.input_file.clone()
    } else {
        None
    };
    
    // For normal modes: Get country format if needed for full scan
    let country_format = if args.mode == Mode::Full {
        if let Some(cc) = &effective_country_code {
            match get_country_format(cc) {
                Ok(format) => Some(format),
                Err(e) => {
                    return Err(anyhow!("Error getting format for country {}: {}", cc, e));
                }
            }
        } else {
            return Err(anyhow!("Country code is required for full mode"));
        }
    } else {
        None
    };
    
    // For any mode except Email: Verify the subnet isn't blacklisted if we have a country code
    if let Some(cc) = &effective_country_code {
        // Only verify if not already in blacklist mode and not in email mode
        if args.mode != Mode::Blacklist && args.mode != Mode::Email {
            // Use the original country code for blacklist verification
            match verify_subnet_for_country(&args.subnet, cc, 3).await {
                Ok(_) => {}, // No success message, just continue silently
                Err(e) => return Err(e),
            }
        }
    }

    // Create channels for work distribution
    let (input_tx, input_rx) = async_channel::bounded(100);
    let (output_tx, output_rx) = async_channel::bounded(100);

    // Create shared counters
    let counters = Arc::new(
        Counters {
            requests: AtomicUsize::new(0),
            success: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
            ratelimits: AtomicUsize::new(0),
            hits: AtomicUsize::new(0)
        }
    );
    
    // Create shared latest hit for display in progress bar
    let latest_hit = Arc::new(TokioMutex::new(None::<String>));

    // Calculate estimated total work for progress bar
    let estimated_total = match args.mode {
        Mode::Quick => {
            // For quick mode, use file estimation
            if let Some(file_path) = &input_file_path {
                let prefix = effective_prefix.clone().unwrap_or_default();
                let suffix = effective_suffix.clone().unwrap_or_default();
                let infix = effective_infix.as_deref();
                
                match calculate_estimate(
                    Mode::Quick,
                    file_path,
                    &prefix,
                    &suffix,
                    infix,
                    None
                ).await {
                    Ok(estimate) => estimate,
                    Err(e) => return Err(anyhow!("Error calculating estimate: {}", e)),
                }
            } else {
                return Err(anyhow!("No input file specified for quick mode"));
            }
        },
        Mode::Email => {
            // For email mode, use a simplified estimate based on file size
            if let Some(file_path) = &input_file_path {
                let metadata = tokio::fs::metadata(file_path).await?;
                let file_size = metadata.len();
                
                // Rough estimate: assume average email is 30 bytes
                let estimated_emails = file_size / 30;
                std::cmp::max(estimated_emails, 100) // At least 100
            } else {
                return Err(anyhow!("No input file specified for email mode"));
            }
        },
        Mode::Full => {
            // For full mode, use country format estimation
            if let Some(format) = &country_format {
                let generator = match PhoneNumberGenerator::new(
                    format,
                    effective_prefix.clone(),
                    effective_suffix.clone(),
                    effective_infix.clone(),
                    args.digits
                ) {
                    Ok(gen) => gen,
                    Err(e) => return Err(anyhow!("Error creating number generator: {}", e)),
                };
                
                generator.estimate_total()
            } else {
                return Err(anyhow!("No country format available for full mode"));
            }
        },
        _ => 100 // Minimal for other modes
    };

    // Create a channel for sending the final estimate
    let (total_tx, total_rx) = async_channel::bounded::<u64>(1);
    
    // Send the initial estimate right away
    let _ = total_tx.send(estimated_total).await;
    
    // Clone args values for the work_queue_handle
    let args_mode = args.mode;
    let args_suffix = effective_suffix.clone().unwrap_or_default();
    let args_country_code = effective_country_code.clone();
    let args_prefix = effective_prefix.clone();
    let args_infix = effective_infix.clone();
    let args_digits = args.digits;
    let args_input_file = input_file_path.clone();
    
    // Create a separate work_queue_handle to populate the input channel based on the mode
    let work_queue_handle = tokio::spawn(async move {
        let result = match args_mode {
            Mode::Quick => {
                // For quick mode, queue from file
                if let Some(file_path) = &args_input_file {
                    // Use prefix filtering for quick mode too
                    let prefix = args_prefix.unwrap_or_default();
                    let suffix = args_suffix;
                    let infix = args_infix.as_deref();
                    queue_from_file(
                        input_tx.clone(), 
                        file_path, 
                        &prefix, // Use the prefix parameter 
                        &suffix,
                        infix
                    ).await
                } else {
                    Err(anyhow!("No input file specified for quick mode"))
                }
            },
            Mode::Email => {
                // For email mode, queue from file (will be handled by main worker)
                if let Some(file_path) = &args_input_file {
                    queue_from_file(
                        input_tx.clone(),
                        file_path,
                        "", // No prefix filtering for emails
                        "", // No suffix filtering for emails
                        None // No infix filtering for emails
                    ).await
                } else {
                    Err(anyhow!("No input file specified for email mode"))
                }
            },
            Mode::Full => {
                // For full mode, generate numbers based on country format
                if let Some(cc) = &args_country_code {
                    match get_country_format(cc) {
                        Ok(format) => {
                            // Create number generator
                            match PhoneNumberGenerator::new(
                                &format,
                                args_prefix,
                                Some(args_suffix),
                                args_infix,
                                args_digits
                            ) {
                                Ok(mut generator) => {
                                    // Generate and queue numbers
                                    while let Some(phone) = generator.next() {
                                        if let Err(error) = input_tx.send(phone.clone()).await {
                                            eprintln!("Failed to send to channel: {}", error);
                                            break;
                                        }
                                    }
                                    Ok(())
                                },
                                Err(e) => Err(anyhow!("Failed to create number generator: {}", e))
                            }
                        },
                        Err(e) => {
                            Err(anyhow!("Country format not found: {}", e))
                        }
                    }
                } else {
                    Err(anyhow!("No country code specified for full mode"))
                }
            },
            _ => {
                Ok(())
            }
        };
        
        if let Err(e) = result {
            eprintln!("Error queueing work: {}", e);
        }

        // Close the channel when all work is queued
        input_tx.close();
    });

    // Clone args values for the worker tasks
    let args_subnet = args.subnet.clone();
    let args_first_name = args.first_name.clone();
    let args_last_name = args.last_name.clone();
    let args_workers = args.workers;
    let args_mode = args.mode;
    let args_lookup_type = args.lookup_type;
    
    // Start the worker tasks
    let mut worker_handles = vec![];
    for _ in 0..args_workers {
        let worker_input_rx = input_rx.clone();
        let worker_output_tx = output_tx.clone();
        let worker_counters = Arc::clone(&counters);
        let worker_subnet = args_subnet.clone();
        let worker_first_name = args_first_name.clone();
        let worker_last_name = args_last_name.clone();
        let worker_mode = args_mode;
        let worker_lookup_type = args_lookup_type.clone();
        
        worker_handles.push(tokio::spawn(
            worker(
                worker_counters, 
                worker_input_rx, 
                worker_output_tx,
                worker_subnet,
                worker_first_name,
                worker_last_name,
                worker_mode,
                worker_lookup_type
            )
        ));
    }

    // Start the output task with shared latest hit
    let output_latest_hit = Arc::clone(&latest_hit);
    let output_handle = tokio::spawn(output_writer(output_rx, output_latest_hit));

    // Create metrics reporting task with progress bars
    tokio::spawn(report_metrics(
        Arc::clone(&counters),
        input_rx.clone(),
        estimated_total,
        total_rx,
        Arc::clone(&latest_hit)
    ));

    // First, await the work queue to complete (all numbers added to channel)
    if let Err(e) = work_queue_handle.await {
        eprintln!("Work queue task failed: {:?}", e);
    }
    
    // Now wait for all workers to complete
    for (i, handle) in worker_handles.into_iter().enumerate() {
        if let Err(e) = handle.await {
            eprintln!("Worker {} failed: {:?}", i, e);
        }
    }

    // Close the output channel and wait for output task to complete
    output_tx.close();
    
    if let Err(e) = output_handle.await {
        eprintln!("Output task failed: {:?}", e);
    }
    
    Ok(())
}