// Copyright (c) 2024 Microsoft Corporation
//
// SPDX-License-Identifier: Apache-2.0
//
// Description: Cloud Hypervisor helper to boot a pod VM.

use anyhow::{anyhow, Context, Result};
use crate::vm::{load_config, TestVm, virtio_fs::{SharedFs, setup_virtio_fs, shutdown_virtiofsd}};
use slog::{debug};
use std::sync::Arc;
use kata_types::config::{hypervisor::register_hypervisor_plugin, hypervisor::TopologyConfigInfo, QemuConfig};
use hypervisor::{
    device::{
        device_manager::{do_handle_device, DeviceManager},
        DeviceConfig,
    }
};
use hypervisor::qemu::Qemu;
use hypervisor::{BlockConfig, VsockConfig};
use std::collections::HashMap;
use hypervisor::Hypervisor;
use tokio::sync::RwLock;

pub const QEMU_HYP: &str = "qemu";
const QEMU_VM_NAME: &str = "qemu-test-vm";
const QEMU_CONFIG_PATH: &str = "/tmp/configuration-qemu-test.toml";

// Helper function to boot a Qemu vm.
pub(crate) async fn setup_test_vm() -> Result<TestVm> {
    debug!(sl!(), "qemu: booting up a test vm");
    
    // Register the hypervisor config plugin
    debug!(sl!(), "qemu: Register CLH plugin");
    let config = Arc::new(QemuConfig::new());
    register_hypervisor_plugin("qemu", config);

    // get the kata configuration toml
    let toml_config = load_config(QEMU_CONFIG_PATH)?;

    let hypervisor_config = toml_config
        .hypervisor
        .get(QEMU_HYP)
        .ok_or_else(|| anyhow!("qemu: failed to get hypervisor config"))
        .context("get hypervisor config")?;

    let hypervisor = Arc::new(Qemu::new());
    hypervisor.set_hypervisor_config(hypervisor_config.clone()).await;

    // prepare vm
    // we do not pass any network namesapce since we dont want any
    let empty_anno_map: HashMap<String, String> = HashMap::new();
    hypervisor.prepare_vm(QEMU_VM_NAME, None, &empty_anno_map).await.context("qemu::prepare test vm")?;

    // We need to add devices before starting the vm
    // instantiate device manager
    let topo_config = TopologyConfigInfo::new(&toml_config);
    let dev_manager = Arc::new(
        RwLock::new(DeviceManager::new(hypervisor.clone(), topo_config.as_ref())
        .await
        .context("qemu::failed to create device manager")?
    ));

    add_vsock_device(dev_manager.clone()).await.context("qemu::adding vsock device")?;

    // If config uses image as vm rootfs, insert it as a disk
    if !hypervisor_config.boot_info.image.is_empty() {
        debug!(sl!(), "qemu::adding vm rootfs");
        let blk_config = BlockConfig {
            path_on_host: hypervisor_config.boot_info.image.clone(),
            is_readonly: true,
            driver_option: hypervisor_config.boot_info.vm_rootfs_driver.clone(),
            ..Default::default()
        };
        add_block_device(dev_manager.clone(), blk_config).await.context("qemu::adding vm rootfs")?;
    }

    // setup file system sharing, if hypervisor supports it
    let mut shared_fs_info = SharedFs::default();
    if hypervisor.capabilities().await?.is_fs_sharing_supported() {
        debug!(sl!(), "qemu::fs sharing is supported, setting it up");
        shared_fs_info = setup_virtio_fs(hypervisor.clone(), dev_manager.clone(), QEMU_VM_NAME).await?;
    }

    // start vm
    hypervisor.start_vm(10_000).await.context("qemu::start vm")?;

    // Qemu only returns the guest_cid in vsock URI
    // append the port information as well
    let mut agent_socket_addr = hypervisor.get_agent_socket().await.context("get agent socket path")?;
    agent_socket_addr.push_str(":1024");

    debug!(sl!(), "qemu: agent socket: {:?}", agent_socket_addr);
    // return the vm structure
    Ok(TestVm{
        hypervisor_name: "qemu".to_string(),
        hypervisor_instance: hypervisor.clone(),
        device_manager: dev_manager.clone(),
        socket_addr: agent_socket_addr,
        is_hybrid_vsock: false,
        shared_fs_info: shared_fs_info,
    })
}

pub(crate) async fn stop_test_vm(instance: Arc<dyn Hypervisor>, fs_info: SharedFs) -> Result<()> {
    debug!(sl!(), "qemu: stopping the test vm");

    if fs_info.pid > 0 {
       shutdown_virtiofsd(fs_info).await?;
    }

    instance.stop_vm().await.context("qemu::stop vm")?;

    Ok(())
}

async fn add_vsock_device(dev_mgr: Arc<RwLock<DeviceManager>>) -> Result<()> {
    let vsock_config = VsockConfig {
        guest_cid: libc::VMADDR_CID_ANY,
    };

    do_handle_device(&dev_mgr, &DeviceConfig::VsockCfg(vsock_config))
        .await
        .context("qemu::handle vsock device failed")?;
    Ok(())
}

async fn add_block_device(dev_mgr: Arc<RwLock<DeviceManager>>, blk_config: BlockConfig) ->Result<()> {
    do_handle_device(&dev_mgr, &DeviceConfig::BlockCfg(blk_config))
        .await
        .context("qemu:handle block device failed")?;
    Ok(())
}
