// Copyright (c) 2024 Microsoft Corporation
//
// SPDX-License-Identifier: Apache-2.0
//
// Description: Cloud Hypervisor helper to boot a pod VM.

use anyhow::{anyhow, Context, Result};
use crate::vm::{load_config, TestVm, virtio_fs::{SharedFs, setup_virtio_fs, shutdown_virtiofsd}};
use slog::debug;
use std::sync::Arc;
use kata_types::config::{hypervisor::register_hypervisor_plugin, hypervisor::HYPERVISOR_NAME_CH, hypervisor::TopologyConfigInfo, CloudHypervisorConfig};
use hypervisor::{
    device::{
        device_manager::{do_handle_device, DeviceManager},
        DeviceConfig,
    }
};
use hypervisor::ch::CloudHypervisor;
use hypervisor::{utils::get_hvsock_path, HybridVsockConfig, DEFAULT_GUEST_VSOCK_CID};
use std::collections::HashMap;
use hypervisor::Hypervisor;
use tokio::sync::RwLock;

pub const CLH_HYP: &str = "clh";
const CLH_VM_NAME: &str = "clh-test-vm";
const CLH_CONFIG_PATH: &str = "/tmp/configuration-clh-test.toml";

// Helper fn to boot up a vm.
// A rough flow for booting a vm.
// - Launch cloud-hypervisor daemon
// - setup api client communication
// - start virtiofsd daemon
// - prepare vm info
// - boot vm using this info
pub(crate) async fn setup_test_vm() -> Result<TestVm> {
    debug!(sl!(), "clh: booting up a test vm");
    
    // Register the hypervisor config plugin
    debug!(sl!(), "clh: Register CLH plugin");
    let config = Arc::new(CloudHypervisorConfig::new());
    register_hypervisor_plugin(HYPERVISOR_NAME_CH, config);

    // get the kata configuration toml
    let toml_config = load_config(CLH_CONFIG_PATH)?;

    let hypervisor_config = toml_config
        .hypervisor
        .get("cloud-hypervisor")
        .ok_or_else(|| anyhow!("clh: failed to get hypervisor config"))
        .context("get hypervisor config")?;

    let hypervisor = Arc::new(CloudHypervisor::new());
    hypervisor.set_hypervisor_config(hypervisor_config.clone()).await;

    // prepare vm
    // we do not pass any network namesapce since we dont want any
    let empty_anno_map: HashMap<String, String> = HashMap::new();
    hypervisor.prepare_vm(CLH_VM_NAME, None, &empty_anno_map).await.context("clh: prepare test vm")?;

    // We need to add devices before starting the vm
    // Handling hvsock device for now
    // instantiate device manager
    let topo_config = TopologyConfigInfo::new(&toml_config);
    let dev_manager = Arc::new(
        RwLock::new(DeviceManager::new(hypervisor.clone(), topo_config.as_ref())
        .await
        .context("clh::failed to create device manager")?
    ));

    // setup file system sharing, if hypervisor supports it
    let mut shared_fs_info = SharedFs::default();
    if hypervisor.capabilities().await?.is_fs_sharing_supported() {
        debug!(sl!(), "clh::fs sharing is supported, setting it up");
        shared_fs_info = setup_virtio_fs(hypervisor.clone(), dev_manager.clone(), CLH_VM_NAME).await?;
    }

    // start vm
    hypervisor.start_vm(10_000).await.context("clh::start vm")?;

    let agent_socket_addr = hypervisor.get_agent_socket().await.context("clh::get agent socket path")?;

    // return the vm structure
    Ok(TestVm{
        hypervisor_name: "clh".to_string(),
        hypervisor_instance: hypervisor.clone(),
        device_manager: dev_manager.clone(),
        socket_addr: agent_socket_addr,
        is_hybrid_vsock: true,
        shared_fs_info: shared_fs_info,
    })
}

pub(crate) async fn stop_test_vm(instance: Arc<dyn Hypervisor>, fs_info: SharedFs) -> Result<()> {
    debug!(sl!(), "clh: stopping the test vm");

    if fs_info.pid > 0 {
       shutdown_virtiofsd(fs_info).await?;
    }

    instance.stop_vm().await.context("clh::stop vm")?;

    Ok(())
}

#[allow(dead_code)]
async fn add_hvsock_device(dev_mgr: Arc<RwLock<DeviceManager>>) -> Result<()> {
    let hvsock_config = HybridVsockConfig {
        guest_cid: DEFAULT_GUEST_VSOCK_CID,
        uds_path: get_hvsock_path(CLH_VM_NAME),
    };

    do_handle_device(&dev_mgr, &DeviceConfig::HybridVsockCfg(hvsock_config))
        .await
        .context("clh::hybrid-vsock device failed")?;

    Ok(())
}
