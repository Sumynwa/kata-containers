// Copyright (c) 2024 Microsoft Corporation
//
// SPDX-License-Identifier: Apache-2.0
//

use anyhow::{Context, Result};
use slog::debug;
use crate::vm::virtio_fs::{VIRTIO_FS, MOUNT_GUEST_TAG};
use kata_sys_util::mount;
use nix::mount::MsFlags;
use protocols::agent::Storage;
use std::fs;

const GUEST_SHARED_PATH: &str = "/run/kata-containers/shared/containers";
const ROOTFS: &str = "rootfs";

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

// Create the container rootfs host share path
fn get_host_share_path(host_path: &str, id: &str) -> String {
    let mut path = host_path.to_string();
    path.push_str("/");
    path.push_str(id);
    path.push_str("/");
    path.push_str(ROOTFS);
    path
}

pub fn share_rootfs(bundle_dir: &str, host_path: &str, id: &str) -> Result<String> {
    debug!(sl!(), "share_rootfs");

    // prepare rootfs string on host
    let rootfs_host_path = get_host_share_path(host_path, id);
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

    let rootfs_host_path = get_host_share_path(host_path, id);
    mount::umount_timeout(&rootfs_host_path, 0).context("unshare_rootfs:: umount rootfs")?;

    if let Ok(md) = fs::metadata(&rootfs_host_path) {
        if md.is_dir() {
            fs::remove_dir(&rootfs_host_path).context("unshare_rootfs:: remove the rootfs mount point as a dir")?;
        }
    }

    Ok(())
}
