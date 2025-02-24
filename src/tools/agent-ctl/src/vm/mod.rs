// Copyright (c) 2024 Microsoft Corporation
//
// SPDX-License-Identifier: Apache-2.0
//
// Description: Boot UVM for testing container storages/volumes.

use anyhow::{anyhow, Context, Result};
use slog::{debug, warn};
use std::sync::Arc;
use hypervisor::Hypervisor;
use hypervisor::device::device_manager::DeviceManager;
use kata_types::config::TomlConfig;
use tokio::sync::RwLock;
use virtio_fs::SharedFs;

mod clh;
mod qemu;
mod virtio_fs;
pub mod utils;

#[derive(Clone)]
pub struct TestVm {
    pub hypervisor_name: String,
    pub hypervisor_instance: Arc<dyn Hypervisor>,
    #[allow(dead_code)]
    pub device_manager: Arc<RwLock<DeviceManager>>,
    pub socket_addr: String,
    pub is_hybrid_vsock: bool,
    pub shared_fs_info: SharedFs,
}

// Helper function to parse a configuration file.
fn load_config(config_file: &str) -> Result<TomlConfig> {
    debug!(sl!(), "test_vm_boot: Load kata configuration file");

    let (mut toml_config, _) = TomlConfig::load_from_file(config_file).context("test_vm_boot:Failed to load kata config file")?;

    // Update the agent kernel params in hypervisor config
    update_agent_kernel_params(&mut toml_config)?;

    // validate configuration and return the error
    toml_config.validate()?;

    debug!(sl!(), "get config content {:?}", &toml_config);
    Ok(toml_config)
}

pub fn to_kernel_string(key:String, val: String) -> Result<String> {
    if key.is_empty() && val.is_empty() {
        Err(anyhow!("Empty key and value"))
    } else if key.is_empty() {
        Err(anyhow!("Empty key"))
    } else if val.is_empty() {
        Ok(key.to_string())
    } else {
        Ok(format!("{}{}{}", key, "=", val))
    }
}

fn update_agent_kernel_params(config: &mut TomlConfig) -> Result<()> {
    let mut params = vec![];
    if let Ok(kv) = config.get_agent_kernel_params() {
        for (k, v) in kv.into_iter() {
            if let Ok(s) = to_kernel_string(k.to_owned(), v.to_owned()) {
                params.push(s);
            }
        }
        if let Some(h) = config.hypervisor.get_mut(&config.runtime.hypervisor_name) {
            h.boot_info.add_kernel_params(params);
        }
    }
    Ok(())
}

// Helper method to boot a test pod VM
pub fn boot_test_vm(hypervisor_name: String) -> Result<TestVm> {
    debug!(sl!(), "boot_test_vm: Booting up a test pod vm with {:?}", hypervisor_name);

    // create a new hypervisor instance
    match hypervisor_name.as_str() {
        clh::CLH_HYP => {
            return tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(clh::setup_test_vm())
                .context("setting up test vm using Cloud Hypervisor");

        }
        qemu::QEMU_HYP => {
            return tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(qemu::setup_test_vm())
                .context("setting up test vm using Qemu");
        }
        _ => {
            warn!(sl!(), "boot_test_vm: Unsupported hypervisor : {:?}", hypervisor_name);
            return Err(anyhow!(
                    "boot_test_vm: Unsupported hypervisor name"
            ));
        }
    }
}

// Helper method to shutdown a test pod VM
pub fn stop_test_vm(vm_instance: TestVm) -> Result<()> {
    debug!(sl!(), "stop_test_vm: stopping booted vm");

    match vm_instance.hypervisor_name.as_str(){
        clh::CLH_HYP => {
            let _ = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(clh::stop_test_vm(vm_instance.hypervisor_instance.clone(), vm_instance.shared_fs_info.clone()))
                .context("stop booted test vm")?;
        }
        qemu::QEMU_HYP => {
            let _ = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(qemu::stop_test_vm(vm_instance.hypervisor_instance.clone(), vm_instance.shared_fs_info.clone()))
                .context("stop booted test vm")?;
        }
        _ => {
            warn!(sl!(), "Invalid hypervisor name passed to shutdown: {:?}", vm_instance.hypervisor_name);
        }
    }

    Ok(())
}

pub fn handle_storages(dev_mgr: Arc<RwLock<DeviceManager>>, storage_list: &str, host_share: String) -> Result<()> {
    debug!(sl!(), "handle_storages");

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(utils::do_handle_storage(dev_mgr.clone(), storage_list, host_share))
        .context("failed to handle storages")
}
