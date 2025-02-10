// Copyright (c) 2024 Microsoft Corporation
//
// SPDX-License-Identifier: Apache-2.0
//
// Description: Cloud Hypervisor helper to boot a pod VM.

use anyhow::{Context, Result};
use slog::{debug};
use std::sync::Arc;
use kata_types::config::hypervisor::Hypervisor as HypervisorConfig;
use hypervisor::{
    device::{
        device_manager::{do_handle_device, DeviceManager},
        DeviceConfig,
    }
};
use hypervisor::ch::CloudHypervisor;
use hypervisor::{utils::get_hvsock_path, HybridVsockConfig, DEFAULT_GUEST_VSOCK_CID};
use kata_types::config::{hypervisor::TopologyConfigInfo, TomlConfig};
use std::collections::HashMap;
use hypervisor::Hypervisor;
use super::TestVm;
use tokio::sync::RwLock;

const TEST_VM_NAME: &str = "clh-test-vm";

// Helper fn to boot up a vm.
// A rough flow for booting a vm.
// - Launch cloud-hypervisor daemon
// - setup api client communication
// - start virtiofsd daemon
// - prepare vm info
// - boot vm using this info
pub(crate) async fn setup_test_vm(config: HypervisorConfig, toml_config: TomlConfig) -> Result<TestVm> {
    debug!(sl!(), "clh: booting up a test vm");
    
    let hypervisor = Arc::new(CloudHypervisor::new());
    hypervisor.set_hypervisor_config(config).await;

    // prepare vm
    // we do not pass any network namesapce since we dont want any
    let empty_anno_map: HashMap<String, String> = HashMap::new();
    hypervisor.prepare_vm(TEST_VM_NAME, None, &empty_anno_map).await.context("prepare test vm")?;

    // We need to add devices before starting the vm
    // Handling hvsock device for now
    // instantiate device manager
    let topo_config = TopologyConfigInfo::new(&toml_config);
    let dev_manager = Arc::new(
        RwLock::new(DeviceManager::new(hypervisor.clone(), topo_config.as_ref())
        .await
        .context("failed to create device manager")?
    ));

    // start vm
    hypervisor.start_vm(10_000).await.context("start vm")?;

    //if hypervisor.capabilities()
    //    .await?
    //    .is_hybrid_vsock_supported() {
    //        add_hvsock_device(dev_manager.clone()).await.context("adding hvsock device")?;
    //} else {
    //    return Err(anyhow!("Hybrid vsock not supported"));
    //}

    let agent_socket_addr = hypervisor.get_agent_socket().await.context("get agent socket path")?;

    // return the vm structure
    Ok(TestVm{
        hypervisor_name: "cloud_hypervisor".to_string(),
        hypervisor_instance: hypervisor.clone(),
        device_manager: dev_manager.clone(),
        socket_addr: agent_socket_addr,
        is_hybrid_vsock: true,
    })
}

pub(crate) async fn stop_test_vm(instance: Arc<dyn Hypervisor>) -> Result<()> {
    debug!(sl!(), "clh: stopping the test vm");

    instance.stop_vm().await.context("stop vm")?;

    Ok(())
}

#[allow(dead_code)]
async fn add_hvsock_device(dev_mgr: Arc<RwLock<DeviceManager>>) -> Result<()> {
    let hvsock_config = HybridVsockConfig {
        guest_cid: DEFAULT_GUEST_VSOCK_CID,
        uds_path: get_hvsock_path(TEST_VM_NAME),
    };

    do_handle_device(&dev_mgr, &DeviceConfig::HybridVsockCfg(hvsock_config))
        .await
        .context("hybrid-vsock device failed")?;

    Ok(())
}
