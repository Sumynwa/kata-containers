// Copyright (c) 2024 Microsoft Corporation
//
// SPDX-License-Identifier: Apache-2.0
//

use anyhow::{anyhow, Context, Result};
use slog::debug;
use std::sync::Arc;
use std::path::Path;
//use hypervisor::Hypervisor;
use hypervisor::{
    device::{
        DeviceType,
        device_manager::{do_handle_device, get_block_driver, DeviceManager},
        DeviceConfig,
    }
};
use hypervisor::BlockConfig;
use crate::utils::generate_random_hex_string;
use crate::vm::virtio_fs::{VIRTIO_FS, MOUNT_GUEST_TAG};
use kata_sys_util::mount;
use nix::mount::MsFlags;
use nix::sys::{stat, stat::SFlag};
use protocols::agent::{CreateContainerRequest, Storage};
use protocols::oci::Mount;
//use std::collections::HashMap;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use std::fs;

const CNT_MNT_BASE: &str = "/tmp/foo";
const GUEST_BASE_PATH: &str = "/run/kata-containers";
const GUEST_SHARED_PATH: &str = "/run/kata-containers/shared/containers";
const ROOTFS: &str = "rootfs";
const TEST_BLK_APPEND: &str = "test-blk-vol";

lazy_static! {
    // A mutable global list to cache requested storages after they have
    // been handled by the hypervisor
    pub static ref STORAGE_INFO: Mutex<Vec<Storage>> = {
        Mutex::new(Vec::new())
    };
    // A mutable global list to cache mount information for the respective volumes
    pub static ref OCI_MOUNTS_INFO: Mutex<Vec<Mount>> = {
        Mutex::new(Vec::new())
    };
    // A mutable global list to umount
    pub static ref UNMOUNT_HOST_INFO: Mutex<Vec<String>> = {
        Mutex::new(Vec::new())
    };
}

// Create host share path
fn get_host_share_path(host_path: &str, id: &str, base: &str) -> String {
    let mut path = host_path.to_string();
    path.push_str("/");
    path.push_str(id);
    path.push_str("/");
    path.push_str(base);
    path
}

// Create guest path
fn generate_path(guest_base: &str, id: &str, suffix: &str) -> String{
    let mut path = guest_base.to_string();
    path.push_str("/");
    path.push_str(id);
    path.push_str("/");
    path.push_str(suffix);
    path
}

async fn do_unmount() -> Result<()> {
    debug!(sl!(), "unmount container shares in host");

    for host_share in UNMOUNT_HOST_INFO.lock().await.iter() {
        mount::umount_timeout(&host_share, 0).context("unshare mounts")?;

        if let Ok(md) = fs::metadata(&host_share) {
            if md.is_dir() {
                fs::remove_dir(&host_share).context("unshare mounts:: failed to remove directory from host")?;
            }
        }
    }

    Ok(())
}

async fn do_append_storage_and_mounts(req: &mut CreateContainerRequest) -> Result<()> {
    debug!(sl!(), "Modify CreateContainerRequest for storages and OCI mounts");

    for s in STORAGE_INFO.lock().await.iter() {
        req.mut_storages().push(s.clone());
    }

    for m in OCI_MOUNTS_INFO.lock().await.iter() {
        req.mut_OCI().mut_Mounts().push(m.clone())
    }

    Ok(())
}

pub fn get_virtiofs_storage() -> Storage {
    Storage {
        driver: String::from(VIRTIO_FS),
        driver_options: Vec::new(),
        source: String::from(MOUNT_GUEST_TAG),
        fstype: String::from("virtiofs"),
        options: vec![String::from("nodev")],
        mount_point: String::from(GUEST_SHARED_PATH),
        ..Default::default()
    }
}

pub fn share_rootfs(bundle_dir: &str, host_path: &str, id: &str) -> Result<String> {
    debug!(sl!(), "share_rootfs");

    // prepare rootfs string on host
    let rootfs_host_path = get_host_share_path(host_path, id, ROOTFS);
    debug!(sl!(), "share_rootfs:: target: {}", rootfs_host_path);

    let mut rootfs_src_path = bundle_dir.to_string();
    rootfs_src_path.push_str("/");
    rootfs_src_path.push_str(ROOTFS);

    // Mount the src path to shared path
    mount::bind_mount_unchecked(&rootfs_src_path, &rootfs_host_path, false, MsFlags::MS_SLAVE)
        .with_context(|| format!("share_rootfs:: failed to bind mount {} to {}", &rootfs_src_path, &rootfs_host_path))?;

    // Return the guest equivalent path
    let mut guest_rootfs_path = String::from(GUEST_SHARED_PATH);
    guest_rootfs_path.push_str("/");
    guest_rootfs_path.push_str(id);

    debug!(sl!(), "share_rootfs:: guest path {}", guest_rootfs_path);

    Ok(guest_rootfs_path)
}

pub fn unshare_rootfs(host_path: &str, id: &str) -> Result<()> {
    debug!(sl!(), "unshare_rootfs");

    let rootfs_host_path = get_host_share_path(host_path, id, ROOTFS);
    mount::umount_timeout(&rootfs_host_path, 0).context("unshare_rootfs:: umount rootfs")?;

    if let Ok(md) = fs::metadata(&rootfs_host_path) {
        if md.is_dir() {
            fs::remove_dir(&rootfs_host_path).context("unshare_rootfs:: remove the rootfs mount point as a dir")?;
        }
    }

    Ok(())
}

pub fn unmount_shares() -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(do_unmount())
        .context("failed to unmount shared mounts on host")
}

pub fn append_storages_and_mounts(req: &mut CreateContainerRequest) -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(do_append_storage_and_mounts(req))
        .context("failed to add storages & mounts info in request")
}

// Handle block base storages
// a. hot plug the device in the vm
// b. fix the storage information
// c. generate the equivalent oci::Mount info
async fn handle_block_volume(
    dev_mgr: &RwLock<DeviceManager>,
    mut vol: Storage,
) -> Result<()> {
    debug!(sl!(), "handle block volume");

    // Check if source is valid
    let valid_block_vol = match stat::stat(vol.source.as_str()) {
        Ok(fstat) => SFlag::from_bits_truncate(fstat.st_mode) == SFlag::S_IFBLK,
        Err(_) => false,
    };

    if !valid_block_vol {
        return Err(anyhow!("Not a block special file: {}", vol.source));
    }

    // Hotplug this into the vm
    let blk_driver = get_block_driver(dev_mgr).await;
    let fstat = stat::stat(vol.source.as_str())?;
    let block_device_config = BlockConfig {
        major: stat::major(fstat.st_rdev) as i64,
        minor: stat::minor(fstat.st_rdev) as i64,
        driver_option: blk_driver,
        ..Default::default()
    };

    // create and insert block device into Kata VM
    let device_info = do_handle_device(dev_mgr, &DeviceConfig::BlockCfg(block_device_config.clone()))
        .await
        .context("do handle device failed.")?;

    // Fix the storage information received in argument
    let mut device_id = String::new();
    if let DeviceType::Block(device) = device_info {
        vol.source = if let Some(pci_path) = device.config.pci_path {
            pci_path.to_string()
        } else {
            return Err(anyhow!("block driver is blk but no pci path exists"));
        };
        device_id = device.device_id;
    }

    // generate a random guest path.
    // we modify the container mount path according to that
    let guest_path = generate_path(GUEST_BASE_PATH, device_id.clone().as_str(), TEST_BLK_APPEND);
    debug!(sl!(), "handle_block_volume: guest_path: {}", guest_path);
    vol.mount_point = guest_path.clone();

    let mount_dest = generate_path(CNT_MNT_BASE, device_id.clone().as_str(), TEST_BLK_APPEND);
    debug!(sl!(), "handle_block_volume: mount dest path: {}", mount_dest);
    // generate the OCI Mount specific to this volume
    let mut mount = Mount::default();
    mount.set_destination(mount_dest);
    mount.set_type(vol.fstype.clone());
    mount.set_source(guest_path);
    mount.set_options(vol.options.clone());

    // now we save these in global arrays
    STORAGE_INFO.lock().await.push(vol);
    OCI_MOUNTS_INFO.lock().await.push(mount);

    Ok(())
}

// Handle storages using share_fs
// a. Bind Mount the source into the host shared path
// b. Generate the equivalent OCI mount info
async fn handle_shared_volume(vol: Storage, host_base_path: String) -> Result<()> {
    debug!(sl!(), "handle_shared_volume");

    // Check if the source is a directory
    let valid_share_vol = match stat::stat(vol.source.as_str()) {
        Ok(fstat) => SFlag::from_bits_truncate(fstat.st_mode) == SFlag::S_IFDIR,
        Err(_) => false,
    };

    if !valid_share_vol {
        return Err(anyhow!("Shared volume is not a valid directory"));
    }

    let file_name = Path::new(&vol.source).file_name().unwrap().to_str().unwrap();
    let random_str = generate_random_hex_string(32 as u32);
    let mut host_share_path = host_base_path.clone();//get_host_share_path(host_base_path.clone().as_str(), random_str.clone().as_str(), share_dir.clone().as_str());
    host_share_path.push_str("/");
    host_share_path.push_str(random_str.clone().as_str());
    host_share_path.push_str("-");
    host_share_path.push_str(file_name);

    debug!(sl!(), "handle_shared_volume:: mounting {} on host source: {}", &vol.source, host_share_path);
    mount::bind_mount_unchecked(&vol.source, &host_share_path, true, MsFlags::MS_SLAVE)
        .with_context(|| format!("handle_shared_volume:: failed to bind mount {} to {}", &vol.source, &host_share_path))?;

    // Generate the guest equivalent path
    let mut guest_path = GUEST_SHARED_PATH.to_string();//generate_path(GUEST_SHARED_PATH, random_str.clone().as_str(), share_dir.clone().as_str());
    guest_path.push_str("/");
    guest_path.push_str(random_str.clone().as_str());
    guest_path.push_str("-");
    guest_path.push_str(file_name);

    let mount_dest = CNT_MNT_BASE.to_string();//generate_path(CNT_MNT_BASE, random_str.clone().as_str(), share_dir.clone().as_str());
    debug!(sl!(), "handle_shared_volume: guest source: {} mount dest path: {}", guest_path, mount_dest);

    // generate the OCI Mount specific to this volume
    let mut mount = Mount::default();
    mount.set_destination(mount_dest);
    mount.set_type(vol.fstype.clone());
    mount.set_source(guest_path);
    mount.set_options(vol.options.clone());

    OCI_MOUNTS_INFO.lock().await.push(mount);
    UNMOUNT_HOST_INFO.lock().await.push(host_share_path);

    Ok(())
}

pub async fn do_handle_storage(
    dev_mgr: Arc<RwLock<DeviceManager>>,
    list_path: &str,
    host_share_path: String,
) -> Result<()> {
    debug!(sl!(), "do_handle_storage");

    let file = fs::File::open(list_path)?;
    let storages: Vec<Storage> = serde_json::from_reader(file)?;

    for storage in storages {
        match storage.driver.as_str() {
            "blk" => {
                debug!(sl!(), "do_handle_storage: block device");
                handle_block_volume(&dev_mgr, storage.clone()).await?;
            }
            "virtio-fs" => {
                debug!(sl!(), "do_handle_storage: virtio-fs share");
                handle_shared_volume(storage.clone(), host_share_path.clone()).await?;
            }
            _ => return Err(anyhow!("{} storage type is not supported", storage.driver)),
        };
    }

    Ok(())
}
