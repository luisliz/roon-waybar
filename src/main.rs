use clap::{Arg, Command};
use roon_api::{Info, RoonApi, Services, transport::{Transport, Control, State}, Parsed, transport::Zone, RoonState};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
enum ConnectionState {
    Connecting,
    Connected,     // Transport service available
    Ready,         // Zones data available
    Disconnected,
}
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::time::{timeout, Duration};
use tokio::net::{UnixListener, UnixStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const APP_NAME: &str = "waybar";
const APP_DISPLAY_NAME: &str = "Waybar module for Roon control";
const APP_VERSION: &str = "0.1.0";
const APP_AUTHOR: &str = "Carlos Eberhardt";
const APP_ID: &str = "com.roon.waybar";
const APP_URL: &str = "https://github.com/your-repo/roon-waybar";

fn create_roon_info() -> Info {
    Info::new(
        APP_NAME.to_string(),
        APP_DISPLAY_NAME,
        APP_VERSION,
        Some(APP_AUTHOR),
        APP_ID,
        Some(APP_URL)
    )
}

#[derive(Debug, Serialize, Deserialize)]
struct IpcRequest {
    command: String,
    action: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct IpcResponse {
    status: String,
    data: Option<serde_json::Value>,
    message: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = Command::new("roon-waybar")
        .version(APP_VERSION)
        .author(APP_AUTHOR)
        .about(APP_DISPLAY_NAME)
        .arg(
            Arg::new("daemon")
                .long("daemon")
                .help("Run as daemon")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("action")
                .help("Action to perform")
                .value_parser(["status", "play", "pause", "playpause", "next", "previous", "setup"])
                .index(1),
        )
        .arg(
            Arg::new("ip")
                .long("ip")
                .help("IP address of Roon Core (for setup mode)")
                .value_name("IP"),
        )
        .get_matches();

    if matches.get_flag("daemon") {
        run_daemon().await?
    } else {
        match matches.get_one::<String>("action") {
            Some(action) => match action.as_str() {
                "status" => client_request("status", None).await?,
                "setup" => {
                    let ip = matches.get_one::<String>("ip").map(|s| s.as_str());
                    setup_authorization(ip).await?
                }
                "play" | "pause" | "playpause" | "next" | "previous" => {
                    client_request("transport", Some(action)).await?
                }
                _ => eprintln!("Unknown action: {}", action),
            },
            None => client_request("status", None).await?,
        }
    }

    Ok(())
}

async fn setup_authorization(ip: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    println!("Setting up Roon authorization...");
    println!("This will wait for you to authorize the extension in Roon.");
    println!("Go to Roon Settings > Extensions and enable 'Waybar module for Roon control'");
    if ip.is_some() {
        println!("Using direct IP connection.");
    } else {
        println!("Using network discovery. If this fails, try: --ip 127.0.0.1");
    }
    println!("Press Ctrl+C to cancel.");
    
    let mut roon = RoonApi::new(create_roon_info());
    
    let config_path = dirs::config_dir()
        .map(|p| p.join("roon-waybar"))
        .unwrap_or_else(|| std::path::Path::new(".").to_path_buf());
    
    std::fs::create_dir_all(&config_path)?;
    let config_path_str = config_path.join("config").to_string_lossy().to_string();
    let config_path_for_save = config_path_str.clone();
    
    let services = Some(vec![Services::Transport(Transport::new())]);
    let provided = HashMap::new();
    
    let get_roon_state = move || -> RoonState {
        if let Some(state) = RoonApi::load_config(&config_path_str, "roonstate").as_object() {
            RoonState {
                paired_core_id: state.get("paired_core_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                tokens: state.get("tokens").and_then(|v| v.as_object())
                    .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
                    .unwrap_or_default(),
            }
        } else {
            RoonState {
                paired_core_id: None,
                tokens: Default::default(),
            }
        }
    };
    
    // Try to connect either via IP or discovery
    let connection_result = match ip {
        Some(ip_addr) => {
            println!("Connecting to Roon Core at {}...", ip_addr);
            use std::net::{IpAddr, Ipv4Addr};
            use std::str::FromStr;
            let ip = IpAddr::V4(Ipv4Addr::from_str(ip_addr)?);
            roon.ws_connect(Box::new(get_roon_state), provided, services, &ip, "9330").await
        }
        None => {
            println!("Discovering Roon Core on network...");
            roon.start_discovery(Box::new(get_roon_state), provided, services).await
        }
    };
    
    if let Some((_handlers, mut core_rx)) = connection_result {
        println!("Waiting for Roon Core discovery and authorization...");
        
        let mut authorized = false;
        
        let mut _transport_service: Option<Transport> = None;
        
        while let Some((core_event, msg)) = core_rx.recv().await {
            println!("Received core event: {:?}", core_event);
            
            // Handle core events to get transport service
            match core_event {
                roon_api::CoreEvent::Discovered(_core, _) => {
                    println!("Core discovered, checking for transport service...");
                    // Core is discovered but might not be registered yet
                }
                roon_api::CoreEvent::Registered(mut core, _token) => {
                    println!("Core registered! Getting transport service...");
                    if let Some(transport) = core.get_transport() {
                        _transport_service = Some(transport.clone());
                        println!("Got transport service, subscribing to zones...");
                        transport.subscribe_zones().await;
                    }
                }
                roon_api::CoreEvent::Lost(_core) => {
                    println!("Core connection lost");
                    break;
                }
                _ => {}
            }
            
            if let Some((_msg_type, parsed)) = msg {
                println!("Received message: {:?} -> {:?}", _msg_type, parsed);
                match parsed {
                    Parsed::RoonState(state) => {
                        println!("✓ Authorization successful! Extension is now paired.");
                        println!("RoonState: {:?}", state);
                        
                        // Save the authorization state
                        let state_json = serde_json::json!({
                            "paired_core_id": state.paired_core_id,
                            "tokens": state.tokens
                        });
                        if let Err(e) = RoonApi::save_config(&config_path_for_save, "roonstate", state_json) {
                            println!("Warning: Failed to save authorization state: {}", e);
                        } else {
                            println!("✓ Authorization state saved to {}/roonstate.json", config_path_for_save);
                        }
                        
                        // Don't break immediately - wait for zones to confirm stable connection
                        authorized = true;
                        
                        // Subscribe to zones to keep the connection alive and get zone data
                        println!("Subscribing to zones...");
                        // We need access to the transport service from the core to subscribe
                        // This might require restructuring how we handle the connection
                    }
                    Parsed::Zones(zones) => {
                        if !zones.is_empty() {
                            println!("✓ Successfully connected! Found {} zone(s):", zones.len());
                            for zone in &zones {
                                println!("  - {}", zone.display_name);
                            }
                            if authorized {
                                println!("✓ Pairing complete and zones received - connection is stable!");
                                break;
                            }
                        } else {
                            println!("Received empty zones list");
                        }
                    }
                    _ => {
                        println!("Other parsed message: {:?}", parsed);
                    }
                }
            } else {
                println!("Core event without message");
            }
        }
        
        if authorized {
            println!("\n🎉 Setup complete! You can now use the waybar module.");
            println!("Try running: ./target/debug/roon-waybar status");
        } else {
            println!("\n❌ Setup failed or was interrupted.");
        }
    } else {
        println!("❌ Failed to start Roon discovery. Make sure Roon Core is running on your network.");
    }
    
    Ok(())
}

struct RoonDaemon {
    zones: Arc<Mutex<Vec<Zone>>>,
    transport: Arc<Mutex<Option<Transport>>>,
    connection_state: Arc<Mutex<ConnectionState>>,
}

fn get_socket_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .or_else(|_| std::env::var("TMPDIR"))
        .unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("roon-waybar.sock")
}

async fn client_request(command: &str, action: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = get_socket_path();
    
    let mut stream = match UnixStream::connect(&socket_path).await {
        Ok(stream) => stream,
        Err(_) => {
            eprintln!("Error: roon-waybar daemon is not running");
            eprintln!("Start it with: roon-waybar --daemon");
            std::process::exit(1);
        }
    };
    
    let request = IpcRequest {
        command: command.to_string(),
        action: action.map(|s| s.to_string()),
    };
    
    let request_json = serde_json::to_string(&request)?;
    stream.write_all(request_json.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    
    let mut buffer = vec![0; 8192];
    let n = stream.read(&mut buffer).await?;
    let response_str = String::from_utf8_lossy(&buffer[..n]);
    
    let response: IpcResponse = serde_json::from_str(&response_str)?;
    
    match response.status.as_str() {
        "ok" => {
            if let Some(data) = response.data {
                println!("{}", data);
            }
        }
        "error" => {
            eprintln!("Error: {}", response.message.unwrap_or("Unknown error".to_string()));
            std::process::exit(1);
        }
        _ => {
            eprintln!("Unexpected response: {}", response_str);
            std::process::exit(1);
        }
    }
    
    Ok(())
}

async fn run_daemon() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting roon-waybar daemon...");
    
    let socket_path = get_socket_path();
    
    // Remove existing socket if it exists
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }
    
    let listener = UnixListener::bind(&socket_path)?;
    println!("Daemon listening on: {:?}", socket_path);
    
    // Initialize daemon state
    let daemon_state = Arc::new(RoonDaemon::new().await?);
    
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let state = daemon_state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, state).await {
                        eprintln!("Error handling client: {}", e);
                    }
                });
            }
            Err(e) => eprintln!("Error accepting connection: {}", e),
        }
    }
}

async fn handle_client(mut stream: UnixStream, daemon: Arc<RoonDaemon>) -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = vec![0; 1024];
    let n = stream.read(&mut buffer).await?;
    let request_str = String::from_utf8_lossy(&buffer[..n]);
    
    let request: IpcRequest = serde_json::from_str(request_str.trim())?;
    
    let response = match request.command.as_str() {
        "status" => daemon.get_status().await,
        "transport" => {
            if let Some(action) = request.action {
                daemon.execute_transport(&action).await
            } else {
                IpcResponse {
                    status: "error".to_string(),
                    data: None,
                    message: Some("Missing action for transport command".to_string()),
                }
            }
        }
        _ => IpcResponse {
            status: "error".to_string(),
            data: None,
            message: Some(format!("Unknown command: {}", request.command)),
        },
    };
    
    let response_json = serde_json::to_string(&response)?;
    stream.write_all(response_json.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    
    Ok(())
}

impl RoonDaemon {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let zones = Arc::new(Mutex::new(Vec::new()));
        let transport = Arc::new(Mutex::new(None));
        let connection_state = Arc::new(Mutex::new(ConnectionState::Connecting));
        
        let daemon = RoonDaemon {
            zones: zones.clone(),
            transport: transport.clone(),
            connection_state: connection_state.clone(),
        };
        
        // Start the persistent connection in a background task
        let zones_clone = zones.clone();
        let transport_clone = transport.clone();
        let connection_state_clone = connection_state.clone();
        
        tokio::spawn(async move {
            let mut retry_count = 0;
            let max_retry_delay = Duration::from_secs(300); // 5 minutes max
            
            loop {
                println!("Daemon: Attempting to connect to Roon... (attempt {})", retry_count + 1);
                
                if let Err(e) = Self::establish_connection(zones_clone.clone(), transport_clone.clone(), connection_state_clone.clone()).await {
                    retry_count += 1;
                    
                    // Exponential backoff: 2^retry_count seconds, capped at max_retry_delay
                    let base_delay = Duration::from_secs(2_u64.pow(retry_count.min(8))); // Cap at 2^8 = 256s
                    let delay = base_delay.min(max_retry_delay);
                    
                    eprintln!("Daemon: Connection failed: {}, retrying in {}s... (attempt {})", 
                        e, delay.as_secs(), retry_count);
                    tokio::time::sleep(delay).await;
                    continue;
                }
                // Connection was successful, reset retry count
                retry_count = 0;
                println!("Daemon: Connection lost, attempting to reconnect...");
                
                // Mark as disconnected
                {
                    let mut state = connection_state_clone.lock().unwrap();
                    *state = ConnectionState::Disconnected;
                }
                
                // Brief pause before reconnection attempt
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        
        Ok(daemon)
    }
    
    async fn establish_connection(
        zones: Arc<Mutex<Vec<Zone>>>,
        transport: Arc<Mutex<Option<Transport>>>,
        connection_state: Arc<Mutex<ConnectionState>>,
    ) -> Result<(), String> {
        let mut roon = RoonApi::new(create_roon_info());
        
        let config_path = dirs::config_dir()
            .map(|p| p.join("roon-waybar"))
            .unwrap_or_else(|| std::path::Path::new(".").to_path_buf());
        
        std::fs::create_dir_all(&config_path).map_err(|e| e.to_string())?;
        let config_path_str = config_path.join("config").to_string_lossy().to_string();
        let config_path_for_save = config_path_str.clone();
        
        let services = Some(vec![Services::Transport(Transport::new())]);
        let provided = HashMap::new();
        
        let get_roon_state = move || -> RoonState {
            let loaded_config = RoonApi::load_config(&config_path_str, "roonstate");
            
            if let Some(state) = loaded_config.as_object() {
                RoonState {
                    paired_core_id: state.get("paired_core_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    tokens: state.get("tokens").and_then(|v| v.as_object())
                        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
                        .unwrap_or_default(),
                }
            } else {
                RoonState {
                    paired_core_id: None,
                    tokens: Default::default(),
                }
            }
        };
        
        // Try to connect directly to localhost if config exists
        let config_file_path_for_check = config_path.join("config").to_string_lossy().to_string();
        let connection_result = if std::path::Path::new(&config_file_path_for_check).exists() {
            println!("Daemon: Found existing auth, connecting directly to localhost");
            use std::net::{IpAddr, Ipv4Addr};
            use std::str::FromStr;
            let ip = IpAddr::V4(Ipv4Addr::from_str("127.0.0.1").map_err(|e| e.to_string())?);
            roon.ws_connect(Box::new(get_roon_state), provided, services, &ip, "9330").await
        } else {
            println!("Daemon: No existing auth found, trying discovery");
            timeout(
                Duration::from_secs(10),
                roon.start_discovery(Box::new(get_roon_state), provided, services)
            ).await.unwrap_or(None)
        };
        
        if let Some((_handlers, mut core_rx)) = connection_result {
            println!("Daemon: Connection established, processing events...");
            println!("FLOW: Starting event processing loop - waiting for core events");
            
            let zones_clone_for_core = zones.clone();
            let transport_clone = transport.clone();
            let config_path_for_save_clone = config_path_for_save.clone();
            
            // Mark as connected (transport available)
            {
                let mut state = connection_state.lock().unwrap();
                *state = ConnectionState::Connected;
                println!("FLOW: Connection state -> Connected (transport available)");
            }
            
            let mut transport_subscribed = false;
            let connection_start = std::time::Instant::now();
            let mut ready_timeout_warned = false;
            let mut zone_refresh_attempts = 0;
            let mut last_zone_check = std::time::Instant::now();
            
            // Process events - this is the persistent connection loop
            while let Some((core_event, msg)) = core_rx.recv().await {
                // Check for Ready state timeout and zone data health
                if !ready_timeout_warned && connection_start.elapsed() > Duration::from_secs(30) {
                    let current_state = connection_state.lock().unwrap().clone();
                    if current_state != ConnectionState::Ready {
                        println!("FLOW: Warning - not Ready after 30s, current state: {:?}", current_state);
                        ready_timeout_warned = true;
                    }
                }
                
                // Periodic zone data validation (every 10 seconds)
                if transport_subscribed && last_zone_check.elapsed() > Duration::from_secs(10) {
                    let current_state = connection_state.lock().unwrap().clone();
                    if current_state == ConnectionState::Connected {
                        // We have transport but no zones yet
                        zone_refresh_attempts += 1;
                        if zone_refresh_attempts <= 3 {
                            println!("FLOW: Zone data missing after subscription, attempt {} to refresh", zone_refresh_attempts);
                            // Note: We can't force a zone refresh easily, but we can log the issue
                            // The Roon API doesn't expose direct zone fetching methods
                        }
                    }
                    last_zone_check = std::time::Instant::now();
                }
                match core_event {
                    roon_api::CoreEvent::Discovered(mut core, _) => {
                        println!("Daemon: Core discovered");
                        println!("FLOW: CoreEvent::Discovered received - transport_subscribed: {}", transport_subscribed);
                        if !transport_subscribed {
                            if let Some(transport_service) = core.get_transport() {
                                {
                                    let mut t = transport_clone.lock().unwrap();
                                    *t = Some(transport_service.clone());
                                }
                                println!("Daemon: Got transport service, subscribing to zones...");
                                println!("FLOW: Calling transport_service.subscribe_zones()");
                                match timeout(Duration::from_secs(15), transport_service.subscribe_zones()).await {
                                    Ok(_) => {
                                        println!("FLOW: subscribe_zones() completed successfully");
                                    }
                                    Err(_) => {
                                        println!("FLOW: subscribe_zones() timed out after 15s - continuing anyway");
                                        // Don't fail completely, zones might arrive later
                                    }
                                }
                                transport_subscribed = true;
                            }
                        }
                    }
                    roon_api::CoreEvent::Registered(mut core, _token) => {
                        println!("Daemon: Core registered!");
                        if !transport_subscribed {
                            if let Some(transport_service) = core.get_transport() {
                                {
                                    let mut t = transport_clone.lock().unwrap();
                                    *t = Some(transport_service.clone());
                                }
                                println!("Daemon: Got transport service, subscribing to zones...");
                                println!("FLOW: Calling transport_service.subscribe_zones()");
                                match timeout(Duration::from_secs(15), transport_service.subscribe_zones()).await {
                                    Ok(_) => {
                                        println!("FLOW: subscribe_zones() completed successfully");
                                    }
                                    Err(_) => {
                                        println!("FLOW: subscribe_zones() timed out after 15s - continuing anyway");
                                        // Don't fail completely, zones might arrive later
                                    }
                                }
                                transport_subscribed = true;
                            }
                        }
                    }
                    roon_api::CoreEvent::Lost(_core) => {
                        println!("Daemon: Core connection lost");
                        break;
                    }
                    _ => {}
                }
                
                // Handle messages
                if let Some((msg_type, parsed)) = msg {
                    println!("FLOW: Message received - type: {:?}", msg_type);
                    match parsed {
                        Parsed::RoonState(state) => {
                            println!("Daemon: Saving new authorization state");
                            let state_json = serde_json::json!({
                                "paired_core_id": state.paired_core_id,
                                "tokens": state.tokens
                            });
                            if let Err(e) = RoonApi::save_config(&config_path_for_save_clone, "roonstate", state_json) {
                                eprintln!("Daemon: Warning: Failed to save authorization state: {}", e);
                            }
                        }
                        Parsed::Zones(zone_list) => {
                            println!("FLOW: Zones message received - count: {}", zone_list.len());
                            
                            // Reset zone refresh attempts since we got data
                            zone_refresh_attempts = 0;
                            
                            // Validate zone data quality
                            let mut valid_zones = 0;
                            for (i, zone) in zone_list.iter().enumerate() {
                                if !zone.zone_id.is_empty() && !zone.display_name.is_empty() {
                                    valid_zones += 1;
                                    if i == 0 {
                                        println!("FLOW: First zone - id: {}, display_name: {}, state: {:?}", 
                                            &zone.zone_id[..8.min(zone.zone_id.len())], zone.display_name, zone.state);
                                        if let Some(ref now_playing) = zone.now_playing {
                                            println!("FLOW: Now playing - track: {}", now_playing.one_line.line1);
                                        } else {
                                            println!("FLOW: No track currently playing");
                                        }
                                    }
                                } else {
                                    println!("FLOW: Warning - zone {} has incomplete data", i);
                                }
                            }
                            
                            println!("FLOW: Zone validation - {}/{} zones have complete data", valid_zones, zone_list.len());
                            {
                                let mut z = zones_clone_for_core.lock().unwrap();
                                *z = zone_list;
                            }
                            println!("Daemon: Updated zone list (zones: {})", zones.lock().unwrap().len());
                            // Mark as ready when we have valid zone data
                            if valid_zones > 0 {
                                let mut state = connection_state.lock().unwrap();
                                *state = ConnectionState::Ready;
                                println!("FLOW: Connection state -> Ready ({} valid zones available)", valid_zones);
                            } else {
                                println!("FLOW: Warning - received zones but none have valid data, staying Connected");
                            }
                        }
                        _ => {
                            match parsed {
                                Parsed::ZonesSeek(ref seeks) if !seeks.is_empty() => {
                                    // Only log every 10th seek to avoid spam
                                    if seeks[0].seek_position.unwrap_or(0) % 10 == 0 {
                                        println!("FLOW: ZonesSeek - position: {:?}, remaining: {}", 
                                            seeks[0].seek_position, seeks[0].queue_time_remaining);
                                    }
                                },
                                _ => {
                                    println!("Daemon: Received other message: {:?}", parsed);
                                }
                            }
                        }
                    }
                }
            }
            
            println!("Daemon: Event loop ended, connection lost");
        } else {
            return Err("Failed to establish connection to Roon server".to_string());
        }
        
        Ok(())
    }
    
    async fn get_status(&self) -> IpcResponse {
        let state = self.connection_state.lock().unwrap().clone();
        println!("FLOW: get_status called - state: {:?}", state);
        
        match state {
            ConnectionState::Connecting => {
                return IpcResponse {
                    status: "ok".to_string(),
                    data: Some(json!({
                        "text": "♪ Connecting...",
                        "tooltip": "Roon: Connecting to server",
                        "class": "connecting"
                    })),
                    message: None,
                };
            }
            ConnectionState::Connected => {
                return IpcResponse {
                    status: "ok".to_string(),
                    data: Some(json!({
                        "text": "♪ Loading zones...",
                        "tooltip": "Roon: Transport connected, loading zones",
                        "class": "loading"
                    })),
                    message: None,
                };
            }
            ConnectionState::Disconnected => {
                return IpcResponse {
                    status: "ok".to_string(),
                    data: Some(json!({
                        "text": "♪ Disconnected",
                        "tooltip": "Roon: Connection lost",
                        "class": "disconnected"
                    })),
                    message: None,
                };
            }
            ConnectionState::Ready => {
                // Continue to zone processing below
            }
        }
        
        let zones = self.zones.lock().unwrap();
        println!("FLOW: get_status - zones.len(): {}", zones.len());
        
        // Smart zone selection: prefer zones with now_playing, then by state priority
        let selected_zone = zones.iter().find(|zone| {
            !zone.zone_id.is_empty() && zone.now_playing.is_some()
        }).or_else(|| {
            zones.iter().find(|zone| !zone.zone_id.is_empty())
        });
        
        if let Some(zone) = selected_zone {
            println!("FLOW: get_status - selected zone: {} (has_track: {})", 
                &zone.zone_id[..8.min(zone.zone_id.len())], zone.now_playing.is_some());
            let text = if let Some(now_playing) = &zone.now_playing {
                format!("♪ {}", now_playing.one_line.line1)
            } else {
                "♪ No Track".to_string()
            };
            
            let tooltip = if let Some(now_playing) = &zone.now_playing {
                format!(
                    "{}\n{}\nZone: {}",
                    now_playing.two_line.line1,
                    now_playing.three_line.line1,
                    zone.display_name
                )
            } else {
                format!("Zone: {}\nNo track playing", zone.display_name)
            };
            
            let class = match zone.state {
                State::Playing => "playing",
                State::Paused => "paused", 
                State::Loading => "loading",
                _ => "stopped",
            };
            
            IpcResponse {
                status: "ok".to_string(),
                data: Some(json!({
                    "text": text,
                    "tooltip": tooltip,
                    "class": class
                })),
                message: None,
            }
        } else {
            IpcResponse {
                status: "ok".to_string(),
                data: Some(json!({
                    "text": "♪ No Zones",
                    "tooltip": "Roon: No zones found",
                    "class": "no-zones"
                })),
                message: None,
            }
        }
    }
    
    async fn execute_transport(&self, action: &str) -> IpcResponse {
        let state = self.connection_state.lock().unwrap().clone();
        println!("FLOW: execute_transport called - action: {}, state: {:?}", action, state);
        
        match state {
            ConnectionState::Ready => {
                // Continue with transport execution below
            }
            _ => {
                return IpcResponse {
                    status: "error".to_string(),
                    data: None,
                    message: Some(format!("Cannot execute transport - state: {:?}", state)),
                };
            }
        }
        
        // Clone the transport and zone data to avoid holding locks across await
        let (transport_clone, zone_clone) = {
            let transport_guard = self.transport.lock().unwrap();
            let zones_guard = self.zones.lock().unwrap();
            
            let transport = match transport_guard.as_ref() {
                Some(t) => t.clone(),
                None => {
                    return IpcResponse {
                        status: "error".to_string(),
                        data: None,
                        message: Some("Transport service not available".to_string()),
                    };
                }
            };
            
            // Smart zone selection for transport - prefer zones with transport controls enabled
            let selected_zone = zones_guard.iter().find(|zone| {
                !zone.zone_id.is_empty() && (
                    zone.is_play_allowed || zone.is_pause_allowed || 
                    zone.is_next_allowed || zone.is_previous_allowed
                )
            }).or_else(|| {
                zones_guard.iter().find(|zone| !zone.zone_id.is_empty())
            });
            
            let zone = match selected_zone {
                Some(z) => z.clone(),
                None => {
                    return IpcResponse {
                        status: "error".to_string(),
                        data: None,
                        message: Some("No valid zones available for transport".to_string()),
                    };
                }
            };
            
            (transport, zone)
        };
        
        let control = match action {
            "play" => Control::Play,
            "pause" => Control::Pause,
            "playpause" => {
                if zone_clone.now_playing.is_none() {
                    return IpcResponse {
                        status: "error".to_string(),
                        data: None,
                        message: Some("No track playing for PlayPause".to_string()),
                    };
                }
                Control::PlayPause
            },
            "next" => {
                if !zone_clone.is_next_allowed {
                    return IpcResponse {
                        status: "error".to_string(),
                        data: None,
                        message: Some("Next is not allowed for this zone".to_string()),
                    };
                }
                Control::Next
            },
            "previous" => {
                if !zone_clone.is_previous_allowed {  
                    return IpcResponse {
                        status: "error".to_string(),
                        data: None,
                        message: Some("Previous is not allowed for this zone".to_string()),
                    };
                }
                Control::Previous
            },
            _ => {
                return IpcResponse {
                    status: "error".to_string(),
                    data: None,
                    message: Some(format!("Unknown action: {}", action)),
                };
            }
        };
        
        // Send the control command
        match timeout(Duration::from_secs(10), transport_clone.control(&zone_clone.zone_id, &control)).await {
            Ok(_) => {
                println!("FLOW: execute_transport - control command sent successfully");
            }
            Err(_) => {
                return IpcResponse {
                    status: "error".to_string(),
                    data: None,
                    message: Some(format!("Transport command '{}' timed out after 10s", action)),
                };
            }
        }
        
        IpcResponse {
            status: "ok".to_string(),
            data: None,
            message: Some(format!("Transport action '{}' executed", action)),
        }
    }
}