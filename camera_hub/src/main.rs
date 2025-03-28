//! Privastead camera hub.
//!
//! Copyright (C) 2024  Ardalan Amiri Sani
//!
//! This program is free software: you can redistribute it and/or modify
//! it under the terms of the GNU General Public License as published by
//! the Free Software Foundation, either version 3 of the License, or
//! (at your option) any later version.
//!
//! This program is distributed in the hope that it will be useful,
//! but WITHOUT ANY WARRANTY; without even the implied warranty of
//! MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//! GNU General Public License for more details.
//!
//! You should have received a copy of the GNU General Public License
//! along with this program.  If not, see <https://www.gnu.org/licenses/>.

#[macro_use]
extern crate log;

#[macro_use]
extern crate serde_derive;

use cfg_if::cfg_if;
use docopt::Docopt;
use image::Luma;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::random::OpenMlsRand;
use openmls_traits::OpenMlsProvider;
use privastead_client_lib::pairing;
use privastead_client_lib::user::{KeyPackages, User};
use privastead_client_lib::video_net_info::{VideoAckInfo, VideoNetInfo};
use qrcode::QrCode;
use rand::Rng;
use serde_yml::Value;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::ops::Add;
use std::path::Path;
use std::process::{exit, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::SystemTime;
use std::{thread, time::Duration};

cfg_if! {
     if #[cfg(all(feature = "ip", feature = "raspberry"))] {
        mod raspberry_pi;
        use crate::raspberry_pi::rpi_camera::RaspberryPiCamera;
        mod ip;
        use crate::ip::ip_camera::IpCamera;
        use rpassword::read_password;
    } else if #[cfg(feature = "raspberry")] {
        mod raspberry_pi;
        use crate::raspberry_pi::rpi_camera::RaspberryPiCamera;
    } else if #[cfg(feature = "ip")] {
        mod ip;
        use crate::ip::ip_camera::IpCamera;
        use rpassword::read_password;
    } else {
        compile_error!("At least one of the features 'raspberry' or 'ip' must be enabled");
    }
}

mod delivery_monitor;

use crate::delivery_monitor::{DeliveryMonitor, VideoInfo};

mod livestream;

use crate::livestream::{is_there_livestream_start_request, livestream};

mod fmp4;
mod mp4;
mod traits;

use crate::traits::Camera;

// Used to generate random names.
// With 16 alphanumeric characters, the probability of collision is very low.
// Note: even if collision happens, it has no impact on
// our security guarantees. Will only cause availability issues.
const NUM_RANDOM_CHARS: u8 = 16;

const STATE_DIR_GENERAL: &str = "state";
const VIDEO_DIR_GENERAL: &str = "pending_videos";

// A counter representing the amount of active camera threads
static GLOBAL_THREAD_COUNT: AtomicUsize = AtomicUsize::new(0);

// Used to ensure there can't be attempted concurrent pairing
static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/*
// Represents the data for each camera
#[derive(Debug, Clone)]
pub struct Camera {
    name: String,
    ip: String,
    rtsp_port: u16,
    username: String,
    password: String,
    state_dir: String,
    video_dir: String,
    motion_fps: u64,
}
*/

fn write_varying_len(stream: &mut TcpStream, msg: &[u8]) {
    // FIXME: is u64 necessary?
    let len = msg.len() as u64;
    let len_data = len.to_be_bytes();

    stream.write_all(&len_data).unwrap();
    stream.write_all(msg).unwrap();
    stream.flush().unwrap();
}

fn read_varying_len(stream: &mut TcpStream) -> Vec<u8> {
    let mut len_data = [0u8; 8];
    stream.read_exact(&mut len_data).unwrap();
    let len = u64::from_be_bytes(len_data);

    let mut msg = vec![0u8; len as usize];
    stream.read_exact(&mut msg).unwrap();

    msg
}

fn perform_pairing_handshake(
    stream: &mut TcpStream,
    camera_key_packages: KeyPackages,
    camera_secret: [u8; pairing::NUM_SECRET_BYTES],
) -> KeyPackages {
    let pairing = pairing::Camera::new(camera_secret, camera_key_packages);

    let app_msg = read_varying_len(stream);
    let (app_key_packages, camera_msg) = pairing.process_app_msg_and_generate_msg_to_app(app_msg);
    write_varying_len(stream, &camera_msg);

    app_key_packages
}

fn generate_camera_secret(camera: &dyn Camera) -> Vec<u8> {
    let crypto = OpenMlsRustCrypto::default();
    let secret = crypto
        .crypto()
        .random_vec(pairing::NUM_SECRET_BYTES)
        .unwrap();

    // Save as QR code to be shown to the app
    let code = QrCode::new(secret.clone()).unwrap();
    let image = code.render::<Luma<u8>>().build();
    image
        .save(format!(
            "camera_{}_secret_qrcode.png",
            camera.get_name().replace(" ", "_").to_lowercase()
        ))
        .unwrap();

    secret
}

#[cfg(feature = "raspberry")]
fn get_input_camera_secret() -> Vec<u8> {
    let pathname = "./camera_secret";
    let file = File::open(pathname).expect(
        "Could not open file \"camera_secret\". You can generate this with the config_tool",
    );
    let mut reader =
        BufReader::with_capacity(file.metadata().unwrap().len().try_into().unwrap(), file);
    let data = reader.fill_buf().unwrap();

    data.to_vec()
}

fn pair_with_app(
    stream: &mut TcpStream,
    camera_key_packages: KeyPackages,
    input_camera_secret: Vec<u8>,
) -> KeyPackages {
    if input_camera_secret.len() != pairing::NUM_SECRET_BYTES {
        panic!("Invalid number of bytes in secret!");
    }

    let mut camera_secret = [0u8; pairing::NUM_SECRET_BYTES];
    camera_secret.copy_from_slice(&input_camera_secret[..]);

    let app_key_packages = perform_pairing_handshake(stream, camera_key_packages, camera_secret);

    app_key_packages
}

fn create_group_and_invite(
    stream: &mut TcpStream,
    camera: &dyn Camera,
    client: &mut User,
    group_name: String,
    app_key_packages: KeyPackages,
) -> io::Result<()> {
    let app_contact = client.add_contact("app".to_string(), app_key_packages)?;
    debug!("Added contact.");

    client.create_group(group_name.clone());
    client.save_groups_state();
    debug!("Created group.");

    let welcome_msg_vec = client.invite(&app_contact, group_name).map_err(|e| {
        error!("invite() returned error:");
        e
    })?;
    client.save_groups_state();
    debug!("App invited to the group.");

    write_varying_len(stream, &welcome_msg_vec);

    File::create(camera.get_state_dir() + "/first_time_done").expect("Could not create file");

    Ok(())
}

fn decrypt_msg(client: &mut User, msg: Vec<u8>) -> io::Result<Vec<u8>> {
    let mut decrypted_msg: Vec<u8> = vec![];

    // FIXME: check contact_name?
    let callback = |msg_bytes: Vec<u8>, _contact_name: String| -> io::Result<()> {
        decrypted_msg = msg_bytes;
        Ok(())
    };

    client.receive_non_ds_single(callback, msg)?;
    client.save_groups_state();

    Ok(decrypted_msg)
}

fn get_wifi_info_and_connect(stream: &mut TcpStream, client: &mut User) -> io::Result<()> {
    let ssid_msg = read_varying_len(stream);
    let ssid_bytes = decrypt_msg(client, ssid_msg)?;
    let ssid = String::from_utf8(ssid_bytes).expect("Invalid UTF-8 for WiFi SSID");
    let password_msg = read_varying_len(stream);
    let password_bytes = decrypt_msg(client, password_msg)?;
    let password = String::from_utf8(password_bytes).expect("Invalid UTF-8 for WiFi password");

    // Disable the Hotspot first
    let _ = Command::new("sh")
        .arg("-c")
        .arg("nmcli connection down id Hotspot")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Wait a bit for Hotspot to get disabled
    thread::sleep(Duration::from_secs(5));

    // Connect to SSID
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "nmcli device wifi connect \"{}\" password \"{}\"",
            ssid.clone(),
            password
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Set up autoconnect to SSID on reboot
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "nmcli connection modify \"{}\" connection.autoconnect yes",
            ssid
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    Ok(())
}

fn create_wifi_hotspot() {
    let _ = Command::new("sh")
        .arg("-c")
        .arg("nmcli device wifi hotspot ssid Privastead password \"12345678\"")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn create_camera_groups(
    camera: &dyn Camera,
    client_motion: &mut User,
    client_livestream: &mut User,
    client_fcm: &mut User,
    client_config: &mut User,
    group_motion_name: String,
    group_livestream_name: String,
    group_fcm_name: String,
    group_config_name: String,
    input_camera_secret: Option<Vec<u8>>,
    connect_to_wifi: bool,
) -> io::Result<()> {
    // Ensure that two cameras don't attempt to pair at the same time (as this would introduce an error when opening two of the same port simultaneously)
    let _lock = LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();

    let secret = if let Some(s) = input_camera_secret.clone() {
        s
    } else {
        generate_camera_secret(camera)
    };

    if input_camera_secret.is_none() {
        println!("[{}] File camera_{}_secret_qrcode.png was just created. Use the QR code in the app to pair.", camera.get_name(), camera.get_name().replace(" ", "_").to_lowercase());
    } else {
        println!("Use the camera QR code in the app to pair.");
    }

    // Wait for the app to connect.
    let listener = TcpListener::bind("0.0.0.0:12348").unwrap();
    let (mut stream, _) = listener.accept().unwrap();

    let app_motion_key_packages =
        pair_with_app(&mut stream, client_motion.key_packages(), secret.clone());
    create_group_and_invite(
        &mut stream,
        camera,
        client_motion,
        group_motion_name,
        app_motion_key_packages,
    )?;

    let app_livestream_key_packages = pair_with_app(
        &mut stream,
        client_livestream.key_packages(),
        secret.clone(),
    );

    create_group_and_invite(
        &mut stream,
        camera,
        client_livestream,
        group_livestream_name,
        app_livestream_key_packages,
    )?;

    let app_fcm_key_packages =
        pair_with_app(&mut stream, client_fcm.key_packages(), secret.clone());
    create_group_and_invite(
        &mut stream,
        camera,
        client_fcm,
        group_fcm_name,
        app_fcm_key_packages,
    )?;

    let app_config_key_packages = pair_with_app(&mut stream, client_config.key_packages(), secret);
    create_group_and_invite(
        &mut stream,
        camera,
        client_config,
        group_config_name,
        app_config_key_packages,
    )?;

    if input_camera_secret.is_none() {
        let _ = fs::remove_file(format!(
            "camera_{}_secret_qrcode.png",
            camera.get_name().replace(" ", "_").to_lowercase()
        ));
    }

    // Send WiFi info to the app.
    if connect_to_wifi {
        get_wifi_info_and_connect(&mut stream, client_config)?;
    }

    Ok(())
}

fn get_names(
    camera: &dyn Camera,
    first_time: bool,
    camera_filename: String,
    group_filename: String,
) -> (String, String) {
    let state_dir = camera.get_state_dir();
    let state_dir_path = Path::new(&state_dir);
    let camera_path = state_dir_path.join(camera_filename);
    let group_path = state_dir_path.join(group_filename);

    let (camera_name, group_name) = if first_time {
        let mut rng = rand::thread_rng();
        let cname: String = (0..NUM_RANDOM_CHARS)
            .map(|_| rng.sample(rand::distributions::Alphanumeric) as char)
            .collect();

        let mut file = File::create(camera_path).expect("Could not create file");
        file.write_all(cname.as_bytes()).unwrap();
        file.flush().unwrap();
        file.sync_all().unwrap();

        //FIXME: how many random characters should we use here?
        let gname: String = (0..NUM_RANDOM_CHARS)
            .map(|_| rng.sample(rand::distributions::Alphanumeric) as char)
            .collect();

        file = File::create(group_path).expect("Could not create file");
        file.write_all(gname.as_bytes()).unwrap();
        file.flush().unwrap();
        file.sync_all().unwrap();

        (cname, gname)
    } else {
        let file = File::open(camera_path).expect("Cannot open file to send");
        let mut reader =
            BufReader::with_capacity(file.metadata().unwrap().len().try_into().unwrap(), file);
        let cname = reader.fill_buf().unwrap();

        let file = File::open(group_path).expect("Cannot open file to send");
        let mut reader =
            BufReader::with_capacity(file.metadata().unwrap().len().try_into().unwrap(), file);
        let gname = reader.fill_buf().unwrap();

        (
            String::from_utf8(cname.to_vec()).unwrap(),
            String::from_utf8(gname.to_vec()).unwrap(),
        )
    };

    (camera_name, group_name)
}

fn get_user_credentials() -> Vec<u8> {
    let pathname = "./user_credentials";
    let file = File::open(pathname).expect(
        "Could not open file \"user_credentials\". You can generate this with the config_tool",
    );
    let mut reader =
        BufReader::with_capacity(file.metadata().unwrap().len().try_into().unwrap(), file);
    let data = reader.fill_buf().unwrap();

    data.to_vec()
}

fn send_motion_triggered_video(
    camera: &dyn Camera,
    client: &mut User,
    group_name: String,
    video_info: VideoInfo,
    delivery_monitor: &mut DeliveryMonitor,
) -> io::Result<()> {
    debug!("Forcing an MLS update.");
    let new_update = client
        .perform_update(group_name.clone())
        .expect("Could not force an MLS update!");
    // We must save state between the calls to perform_update() and send_update().
    // This is to make sure we don't end up sending an update to the app, which
    // we have not successfully committed/saved on our end.
    client.save_groups_state();
    client
        .send_update(group_name.clone())
        .expect("Could not send the pending update!");
    if !new_update {
        // We don't want the attacker to force us to send more than one video without an update.
        // We add the video to the delivery monitor, hoping that it will be sent in the future
        // after the app acks the update.
        info!("Sent pending update. Will not send video until update is acked (indirectly).");
        delivery_monitor.send_event(video_info);
        return Ok(());
    }

    let video_dir = camera.get_video_dir();
    let video_dir_path = Path::new(&video_dir);
    let video_file = video_dir_path.join(&video_info.filename);

    debug!("Starting to send video.");
    let file = File::open(video_file).expect("Cannot open file to send");
    let file_len = file.metadata().unwrap().len();

    // We want each encrypted message to fit within one TCP packet (max size: 64 kB or 65535 B).
    // With these numbers, some experiments show that the encrypted message will have the max
    // size of 64687 B.
    const READ_SIZE: usize = 63 * 1024;
    let mut reader = BufReader::with_capacity(READ_SIZE, file);

    let net_info = VideoNetInfo::new(video_info.timestamp, file_len, READ_SIZE as u64);

    client
        .send(&bincode::serialize(&net_info).unwrap(), group_name.clone())
        .map_err(|e| {
            error!("send() returned error:");
            e
        })?;

    for i in 0..net_info.num_msg {
        let buffer = reader.fill_buf().unwrap();
        let length = buffer.len();
        // Sanity checks
        if i < (net_info.num_msg - 1) {
            assert_eq!(length, READ_SIZE);
        } else {
            assert_eq!(
                length,
                <u64 as TryInto<usize>>::try_into(file_len).unwrap() % READ_SIZE
            );
        }

        client.send(buffer, group_name.clone()).map_err(|e| {
            error!("send_video() returned error:");
            client.save_groups_state();
            e
        })?;
        reader.consume(length);
    }
    client.save_groups_state();

    info!("Sending the video ({}).", video_info.timestamp);
    delivery_monitor.send_event(video_info);
    info!("Sent the video.");

    Ok(())
}

fn process_motion_acks(
    client: &mut User,
    delivery_monitor: &mut DeliveryMonitor,
) -> io::Result<bool> {
    let mut any_ack = false;
    //FIXME: check the contact_name.
    let callback = |msg_bytes: Vec<u8>, _contact_name: String| -> io::Result<()> {
        let acked_videos: Vec<VideoAckInfo> = match bincode::deserialize(&msg_bytes) {
            Ok(acked) => acked,
            Err(e) => {
                error!(
                    "Error: could not convert msg_bytes to vec<u64> for acked videos: {}",
                    e
                );
                return Ok(());
            }
        };

        for video_ack_info in acked_videos {
            info!("Acked: {}", video_ack_info.timestamp);
            delivery_monitor.ack_event(video_ack_info.timestamp, video_ack_info.video_ack);
            any_ack = true;
        }

        Ok(())
    };

    client.receive(callback)?;
    client.save_groups_state();

    Ok(any_ack)
}

fn send_video_notification(
    client: &mut User,
    group_name: String,
    video_info: VideoInfo,
    delivery_monitor: &mut DeliveryMonitor,
) -> io::Result<()> {
    // FIXME: We might send a whole bunch of notifications without forcing
    // an update. If the update is not acked, then we should start sending
    // dummy notifications.
    debug!("An MLS update reminder.");
    client
        .perform_update(group_name.clone())
        .expect("Could not force an MLS update!");
    // We must save state between the calls to perform_update() and send_update().
    // This is to make sure we don't end up sending an update to the app, which
    // we have not successfully committed/saved on our end.
    client.save_groups_state();
    client
        .send_update(group_name.clone())
        .expect("Could not send the pending update!");

    let info_notify = VideoNetInfo::new_notification(video_info.timestamp);

    client
        .send(
            &bincode::serialize(&info_notify).unwrap(),
            group_name.clone(),
        )
        .map_err(|e| {
            error!("send() returned error:");
            e
        })?;
    client.save_groups_state();

    info!("Sending notification for video ({}).", video_info.timestamp);
    delivery_monitor.notify_event(video_info);

    Ok(())
}

const USAGE: &str = "
Privastead camera hub: connects to an IP camera and send videos to the privastead app end-to-end encrypted (through an untrusted server).

Usage:
  privastead-camera-hub
  privastead-camera-hub --reset
  privastead-camera-hub (--version | -v)
  privastead-camera-hub (--help | -h)

Options:
    --reset                                     Wipe all the state
    --version, -v                               Show version
    --help, -h                                  Show help
";

#[derive(Debug, Clone, Deserialize)]
struct Args {
    flag_reset: bool,
}

fn main() -> io::Result<()> {
    let version = env!("CARGO_PKG_NAME").to_string() + ", version: " + env!("CARGO_PKG_VERSION");
    env_logger::init();

    let args: Args = Docopt::new(USAGE)
        .map(|d| d.help(true))
        .map(|d| d.version(Some(version)))
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());

    let credentials = get_user_credentials();

    // Retrieve the cameras.yaml file. If it doesn't exist, print an error message for the user.
    let cameras_file = match File::open("cameras.yaml") {
        Ok(file) => file,

        Err(_error) => {
            println!("Error retrieving cameras.yaml file, see the example_cameras.yaml for an example configuration.");
            exit(1);
        }
    };

    // Load the yml file in for analysis
    let loaded_cameras: HashMap<String, Value> = serde_yml::from_reader(cameras_file)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    // Extract the server IP and cameras
    let server_section = loaded_cameras
        .get("server")
        .expect("Server section is missing from cameras.yaml");
    let cameras_section = loaded_cameras
        .get("cameras")
        .expect("Cameras section is missing from cameras.yaml");
    let server_ip = server_section
        .get("ip")
        .expect("Missing IP for server")
        .as_str()
        .unwrap();

    // Create the general outer directories (where we'll have inner directories representing each camera)
    fs::create_dir_all(STATE_DIR_GENERAL).unwrap();
    fs::create_dir_all(VIDEO_DIR_GENERAL).unwrap();

    let mut camera_list: Vec<Box<dyn Camera + Send>> = Vec::new();
    let delivery_service_addr: String = server_ip.to_owned() + ":12346";

    cfg_if! {
        if #[cfg(feature = "raspberry")] {
            let mut input_camera_secret: Option<Vec<u8>> = None;
            let mut connect_to_wifi = false;
        } else {
            let input_camera_secret: Option<Vec<u8>> = None;
            let connect_to_wifi = false;
        }
    }

    #[cfg(feature = "raspberry")]
    let mut num_raspberry_pi = 0;

    // Iterate through every camera in the cameras.yaml file, accumulating structs representing their data
    if let Value::Sequence(cameras) = cameras_section {
        for camera in cameras {
            if let Value::Mapping(map) = camera {
                let camera_type = map
                    .get(&Value::String("type".to_string()))
                    .expect("Missing camera type (IP or RaspberryPi)")
                    .as_str()
                    .unwrap();
                let camera_name = map
                    .get(&Value::String("name".to_string()))
                    .expect("Missing camera name")
                    .as_str()
                    .unwrap();
                let camera_motion_fps = map
                    .get(&Value::String("motion_fps".to_string()))
                    .expect("Missing Motion FPS")
                    .as_u64()
                    .unwrap();

                if camera_type == "IP" {
                    cfg_if! {
                           if #[cfg(feature = "ip")] {
                            let camera_ip = map
                                .get(&Value::String("ip".to_string()))
                                .expect("Missing IP for camera")
                                .as_str()
                                .unwrap();
                            let camera_rtsp_port = map
                                .get(&Value::String("rtsp_port".to_string()))
                                .expect("Missing RTSP port")
                                .as_u64()
                                .unwrap() as u16;
                            let mut camera_username = map
                                .get(&Value::String("username".to_string()))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let mut camera_password = map
                                .get(&Value::String("password".to_string()))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();

                            if camera_username.is_empty() {
                                camera_username = ask_user(format!(
                                    "Enter the username for the IP camera {:?}: ",
                                    camera_name
                                ))
                                    .unwrap();
                            }

                            if camera_password.is_empty() {
                                camera_password = ask_user_password(format!(
                                    "Enter the password for the IP camera {:?}: ",
                                    camera_name
                                ))
                                    .unwrap();
                            }

                            let ip_camera_result = IpCamera::new(
                                camera_name.parse().unwrap(),
                                camera_ip.parse().unwrap(),
                                camera_rtsp_port,
                                camera_username.parse().unwrap(),
                                camera_password.parse().unwrap(),
                                format!(
                                    "{}/{}",
                                    STATE_DIR_GENERAL,
                                    camera_name.replace(" ", "_").to_lowercase()
                                ),
                                format!(
                                    "{}/{}",
                                    VIDEO_DIR_GENERAL,
                                    camera_name.replace(" ", "_").to_lowercase()
                                ),
                                camera_motion_fps,
                            );
                            match ip_camera_result {
                                Ok(camera) => {
                                    camera_list.push(Box::new(camera));
                                }
                                Err(err) => {
                                    panic!("Failed to initialize the IP camera object. Consider resetting the camera. (Error: {err})");
                                }
                            }

                           } else {
                             panic!("IP cameras are only supported with the \"ip\" feature.");
                        }
                    }
                } else if camera_type == "RaspberryPi" {
                    cfg_if! {
                       if #[cfg(feature = "raspberry")] {
                            if num_raspberry_pi > 0 {
                                panic!("cameras.yaml can only specify for Raspberry Pi camera!");
                            }
                            num_raspberry_pi += 1;

                            let camera = RaspberryPiCamera::new(
                                camera_name.parse().unwrap(),
                                STATE_DIR_GENERAL.to_string(),
                                VIDEO_DIR_GENERAL.to_string(),
                                camera_motion_fps,
                            );
                            camera_list.push(Box::new(camera));

                            input_camera_secret = Some(get_input_camera_secret());
                            connect_to_wifi = true;
                        } else {
                            panic!(
                                "Raspberry Pi cameras are only supported with the \"raspberry\" feature."
                            )
                        }
                    }
                } else {
                    panic!(
                        "Unknown camera type ({:?}). Supported types are IP and RaspberryPi",
                        camera_type
                    )
                };
            }
        }
    }

    // Iterate through each camera struct and spawn in a thread to manage each individual one
    for mut camera in camera_list.into_iter() {
        println!("Starting to instantiate camera: {:?}", camera.get_name());

        let delivery_service_addr = delivery_service_addr.clone();
        let credentials = credentials.clone();
        let args = args.clone();
        let input_camera_secret = input_camera_secret.clone();

        GLOBAL_THREAD_COUNT.fetch_add(1, Ordering::SeqCst);
        thread::spawn(move || {
            loop {
                if args.flag_reset {
                    match reset(
                        camera.as_ref(),
                        delivery_service_addr.clone(),
                        credentials.clone(),
                    ) {
                        Ok(_) => {}
                        Err(e) => {
                            panic!("reset() returned with: {e}");
                        }
                    };

                    // Deduct one from our thread count for main thread to know when to exit (when all are finished)
                    GLOBAL_THREAD_COUNT.fetch_sub(1, Ordering::SeqCst);
                    return;
                } else {
                    match core(
                        camera.as_mut(),
                        delivery_service_addr.clone(),
                        credentials.clone(),
                        input_camera_secret.clone(),
                        connect_to_wifi,
                    ) {
                        Ok(_) => {}
                        Err(e) => {
                            error!("core() returned with: {e}");
                        }
                    }
                }
                println!("There was a problem with the connection to the server. Will try to connect again soon.");
                let _ = fs::remove_file(camera.get_state_dir() + "/registration_done");
                sleep(Duration::from_secs(10));
            }
        });
    }

    // Terminate when no cameras are left running
    while GLOBAL_THREAD_COUNT.load(Ordering::SeqCst) != 0 {
        sleep(Duration::from_millis(1));
    }

    Ok(())
}

/// Helper function to try connecting to the TCP server.
fn try_connect(camera: &dyn Camera, address: &str) -> io::Result<TcpStream> {
    match TcpStream::connect(address) {
        Ok(stream) => Ok(stream),
        Err(e) => {
            println!("Error: failed to connect to the delivery service!");
            let state_dir = camera.get_state_dir();
            let state_dir_path = Path::new(&state_dir);
            let registration_done_path = state_dir_path.join("registration_done");
            let _ = fs::remove_file(registration_done_path);
            Err(e)
        }
    }
}

#[cfg(feature = "ip")]
fn ask_user(prompt: String) -> io::Result<String> {
    print!("{prompt}");
    // Make sure the prompt is displayed before reading input
    io::stdout().flush()?;

    let mut user_input = String::new();
    io::stdin().read_line(&mut user_input)?;
    // Trim the input to remove any extra whitespace or newline characters
    Ok(user_input.trim().to_string())
}

#[cfg(feature = "ip")]
fn ask_user_password(prompt: String) -> io::Result<String> {
    print!("{prompt}");
    // Make sure the prompt is displayed before reading input
    io::stdout().flush()?;

    let password = read_password()?;
    // Trim the input to remove any extra whitespace or newline characters
    Ok(password.trim().to_string())
}

fn reset(
    camera: &dyn Camera,
    delivery_service_addr: String,
    credentials: Vec<u8>,
) -> io::Result<()> {
    // First, deregister from the server
    // FIXME: has some code copy/pasted from core()
    let state_dir = camera.get_state_dir();
    let state_dir_clone = state_dir.clone();
    let state_dir_path = Path::new(&state_dir_clone);
    let first_time_done_path = state_dir_path.join("first_time_done");
    println!("{:?}", first_time_done_path);
    let first_time: bool = !first_time_done_path.exists();

    if first_time {
        println!("There's no state to reset!");
        return Ok(());
    }

    let reregister = false;

    let (camera_motion_name, _group_motion_name) = get_names(
        camera,
        first_time,
        "camera_motion_name".to_string(),
        "group_motion_name".to_string(),
    );

    let (camera_livestream_name, _group_livestream_name) = get_names(
        camera,
        first_time,
        "camera_livestream_name".to_string(),
        "group_livestream_name".to_string(),
    );

    let (camera_fcm_name, _group_fcm_name) = get_names(
        camera,
        first_time,
        "camera_fcm_name".to_string(),
        "group_fcm_name".to_string(),
    );

    let (camera_config_name, _group_config_name) = get_names(
        camera,
        first_time,
        "camera_config_name".to_string(),
        "group_config_name".to_string(),
    );

    let server_motion_stream = try_connect(camera, &delivery_service_addr)?;
    let server_livestream_stream = try_connect(camera, &delivery_service_addr)?;
    let server_fcm_stream = try_connect(camera, &delivery_service_addr)?;
    let server_config_stream = try_connect(camera, &delivery_service_addr)?;

    match User::new(
        camera_motion_name,
        Some(server_motion_stream),
        first_time,
        reregister,
        state_dir.clone(),
        "motion".to_string(),
        credentials.clone(),
        false,
    ) {
        Ok(mut client) => match client.deregister() {
            Ok(_) => {
                info!("Motion client deregistered successfully.")
            }
            Err(e) => {
                error!("Error: Deregistering client_motion failed: {e}");
            }
        },
        Err(e) => {
            error!("Error: Creating client_motion failed: {e}");
        }
    };

    match User::new(
        camera_livestream_name,
        Some(server_livestream_stream),
        first_time,
        reregister,
        state_dir.clone(),
        "livestream".to_string(),
        credentials.clone(),
        false,
    ) {
        Ok(mut client) => match client.deregister() {
            Ok(_) => {
                info!("Livestream client deregistered successfully.")
            }
            Err(e) => {
                error!("Error: Deregistering client_livestream failed: {e}");
            }
        },
        Err(e) => {
            error!("Error: Creating client_livestream failed: {e}");
        }
    };

    match User::new(
        camera_fcm_name,
        Some(server_fcm_stream),
        first_time,
        reregister,
        state_dir.clone(),
        "fcm".to_string(),
        credentials.clone(),
        false,
    ) {
        Ok(mut client) => match client.deregister() {
            Ok(_) => {
                info!("FCM client deregistered successfully.")
            }
            Err(e) => {
                error!("Error: Deregistering client_fcm failed: {e}");
            }
        },
        Err(e) => {
            error!("Error: Creating client_fcm failed: {e}");
        }
    };

    match User::new(
        camera_config_name,
        Some(server_config_stream),
        first_time,
        reregister,
        state_dir,
        "config".to_string(),
        credentials,
        false,
    ) {
        Ok(mut client) => match client.deregister() {
            Ok(_) => {
                info!("Config client deregistered successfully.")
            }
            Err(e) => {
                error!("Error: Deregistering client_config failed: {e}");
            }
        },
        Err(e) => {
            error!("Error: Creating client_config failed: {e}");
        }
    };

    //Second, delete all the local state files.
    let _ = fs::remove_dir_all(state_dir_path);

    //Third, delete all the pending videos (those that were never successfully delivered)
    let video_dir = camera.get_video_dir();
    let video_dir_path = Path::new(&video_dir);
    let _ = fs::remove_dir_all(video_dir_path);

    println!("Reset finished.");
    Ok(())
}

fn core(
    camera: &mut dyn Camera,
    delivery_service_addr: String,
    credentials: Vec<u8>,
    input_camera_secret: Option<Vec<u8>>,
    connect_to_wifi: bool,
) -> io::Result<()> {
    let state_dir = camera.get_state_dir();
    let mut first_time: bool = !Path::new(&(state_dir.clone() + "/first_time_done")).exists();
    let reregister: bool = !Path::new(&(state_dir.clone() + "/registration_done")).exists();

    if first_time && connect_to_wifi {
        create_wifi_hotspot();
    }

    let (camera_motion_name, group_motion_name) = get_names(
        camera,
        first_time,
        "camera_motion_name".to_string(),
        "group_motion_name".to_string(),
    );
    debug!("camera_motion_name = {}", camera_motion_name);
    debug!("group_motion_name = {}", group_motion_name);

    let (camera_livestream_name, group_livestream_name) = get_names(
        camera,
        first_time,
        "camera_livestream_name".to_string(),
        "group_livestream_name".to_string(),
    );
    debug!("camera_livestream_name = {}", camera_livestream_name);
    debug!("group_livestream_name = {}", group_livestream_name);

    let (camera_fcm_name, group_fcm_name) = get_names(
        camera,
        first_time,
        "camera_fcm_name".to_string(),
        "group_fcm_name".to_string(),
    );
    debug!("camera_fcm_name = {}", camera_fcm_name);
    debug!("group_fcm_name = {}", group_fcm_name);

    let (camera_config_name, group_config_name) = get_names(
        camera,
        first_time,
        "camera_config_name".to_string(),
        "group_config_name".to_string(),
    );
    debug!("camera_config_name = {}", camera_config_name);
    debug!("group_config_name = {}", group_config_name);

    let mut client_motion = User::new(
        camera_motion_name.clone(),
        None,
        first_time,
        reregister,
        state_dir.clone(),
        "motion".to_string(),
        credentials.clone(),
        true,
    )
    .map_err(|e| {
        error!("User::new() returned error:");
        e
    })?;
    debug!("Motion client created.");

    let mut client_livestream = User::new(
        camera_livestream_name.clone(),
        None,
        first_time,
        reregister,
        state_dir.clone(),
        "livestream".to_string(),
        credentials.clone(),
        true,
    )
    .map_err(|e| {
        error!("User::new() returned error:");
        e
    })?;
    debug!("Livestream client created.");

    let mut client_fcm = User::new(
        camera_fcm_name.clone(),
        None,
        first_time,
        reregister,
        state_dir.clone(),
        "fcm".to_string(),
        credentials.clone(),
        true,
    )
    .map_err(|e| {
        error!("User::new() returned error:");
        e
    })?;
    debug!("FCM client created.");

    let mut client_config = User::new(
        camera_config_name.clone(),
        None,
        first_time,
        reregister,
        state_dir.clone(),
        "config".to_string(),
        credentials.clone(),
        true,
    )
    .map_err(|e| {
        error!("User::new() returned error:");
        e
    })?;
    debug!("Config client created.");

    let camera_name = camera.get_name();
    if first_time {
        println!(
            "[{}] Waiting to be paired with the mobile app.",
            camera_name
        );
        create_camera_groups(
            camera,
            &mut client_motion,
            &mut client_livestream,
            &mut client_fcm,
            &mut client_config,
            group_motion_name.clone(),
            group_livestream_name.clone(),
            group_fcm_name.clone(),
            group_config_name.clone(),
            input_camera_secret,
            connect_to_wifi,
        )?;
        println!("[{}] Pairing successful.", camera_name);
    }

    // Now, we have access to Internet and we can connect to the delivery service
    let server_motion_stream = try_connect(camera, &delivery_service_addr)?;
    let server_livestream_stream = try_connect(camera, &delivery_service_addr)?;
    let server_fcm_stream = try_connect(camera, &delivery_service_addr)?;
    let server_config_stream = try_connect(camera, &delivery_service_addr)?;
    first_time = false;

    let mut client_motion = User::new(
        camera_motion_name,
        Some(server_motion_stream),
        first_time,
        reregister,
        state_dir.clone(),
        "motion".to_string(),
        credentials.clone(),
        true,
    )
    .map_err(|e| {
        error!("User::new() returned error:");
        e
    })?;
    debug!("Motion client created.");

    let mut client_livestream = User::new(
        camera_livestream_name,
        Some(server_livestream_stream),
        first_time,
        reregister,
        state_dir.clone(),
        "livestream".to_string(),
        credentials.clone(),
        true,
    )
    .map_err(|e| {
        error!("User::new() returned error:");
        e
    })?;
    let mut client_fcm = User::new(
        camera_fcm_name,
        Some(server_fcm_stream),
        first_time,
        reregister,
        state_dir.clone(),
        "fcm".to_string(),
        credentials.clone(),
        true,
    )
    .map_err(|e| {
        error!("User::new() returned error:");
        e
    })?;
    debug!("FCM client created.");

    let _client_config = User::new(
        camera_config_name,
        Some(server_config_stream),
        first_time,
        reregister,
        state_dir.clone(),
        "config".to_string(),
        credentials,
        true,
    )
    .map_err(|e| {
        error!("User::new() returned error:");
        e
    })?;
    debug!("Config client created.");

    fs::File::create(state_dir.clone() + "/registration_done").expect("Could not create file");

    println!("[{}] Running...", camera_name);

    let mut locked_motion_check_time: Option<SystemTime> = None;
    let mut locked_delivery_check_time: Option<SystemTime> = None;
    let video_dir = camera.get_video_dir();
    let mut delivery_monitor = DeliveryMonitor::from_file_or_new(video_dir, state_dir, 60);

    // Used for anti-dither for motion detection
    loop {
        // Check motion events from the IP camera every second
        let motion_event = match camera.is_there_motion() {
            Ok(event) => event,
            Err(e) => {
                println!("Motion detection error {}", e);
                continue;
            }
        };

        // Send motion events only if we haven't sent one in the past minute
        if motion_event
            && (locked_motion_check_time.is_none()
                || locked_motion_check_time.unwrap().le(&SystemTime::now()))
        {
            let video_info = VideoInfo::new();
            info!("Sending the FCM notification with timestamp.");
            client_fcm.send_fcm(
                &bincode::serialize(&video_info.timestamp).unwrap(),
                group_fcm_name.clone(),
            )?;
            client_fcm.save_groups_state();
            match camera.record_motion_video(&video_info) {
                Ok(_) => {
                    info!("Sending the FCM notification to start downloading.");
                    //Timestamp of 0 tells the app it's time to start downloading.
                    let dummy_timestamp: u64 = 0;
                    client_fcm.send_fcm(
                        &bincode::serialize(&dummy_timestamp).unwrap(),
                        group_fcm_name.clone(),
                    )?;
                    client_fcm.save_groups_state();
                    send_motion_triggered_video(
                        camera,
                        &mut client_motion,
                        group_motion_name.clone(),
                        video_info,
                        &mut delivery_monitor,
                    )?;
                    locked_motion_check_time = Some(SystemTime::now().add(Duration::from_secs(60)));
                }
                Err(e) => {
                    error!("Error recording motion video: {e}");
                }
            }
        }

        // Livestream request? Start it.
        if is_there_livestream_start_request(&mut client_livestream)? {
            livestream(
                &mut client_livestream,
                group_livestream_name.clone(),
                camera,
            )?;
        }

        // Process motion acks
        let any_ack = process_motion_acks(&mut client_motion, &mut delivery_monitor)?;

        // Check with the delivery service every minute
        if any_ack
            || (locked_delivery_check_time.is_none()
                || locked_delivery_check_time.unwrap().le(&SystemTime::now()))
        {
            let (resend_list, renotify_list) = delivery_monitor.videos_to_resend_renotify();

            if !resend_list.is_empty() {
                send_motion_triggered_video(
                    camera,
                    &mut client_motion,
                    group_motion_name.clone(),
                    resend_list[0].clone(),
                    &mut delivery_monitor,
                )?;

                for video_info in &resend_list[1..] {
                    delivery_monitor.send_event(video_info.clone());
                }
            }

            // If we resend any videos above, that ends up sending a notification
            // to the app anyway.
            if resend_list.is_empty() && !renotify_list.is_empty() {
                // It's enough to send one notification
                // We just want to send an FCM message in order to get the app to fetch the messages.
                debug!("Sending the FCM notification.");
                let dummy_timestamp: u64 = 0;
                client_fcm.send_fcm(
                    &bincode::serialize(&dummy_timestamp).unwrap(),
                    group_fcm_name.clone(),
                )?;
                client_fcm.save_groups_state();
                send_video_notification(
                    &mut client_motion,
                    group_motion_name.clone(),
                    renotify_list[0].clone(),
                    &mut delivery_monitor,
                )?;

                // For the rest, just tell the delivery_monitor that we sent a notification.
                for video_info in &renotify_list[1..] {
                    delivery_monitor.notify_event(video_info.clone());
                }
            }

            locked_delivery_check_time = Some(SystemTime::now().add(Duration::from_secs(60)));
        }

        sleep(Duration::from_millis(10)); // Introduce a small delay since we don't need this constantly checked
    }
}
