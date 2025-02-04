// Copyright (c) 2024 Microsoft Corporation
//
// SPDX-License-Identifier: Apache-2.0
//
// Description: Helper to boot a pod VM for testing container storages/volumes.

use anyhow::{anyhow, Context, Result};
//use std::path::{Path, PathBuf};
use slog::{debug, warn};
use std::sync::Arc;
use hypervisor::Hypervisor;
use kata_types::config::{TomlConfig, hypervisor::register_hypervisor_plugin, hypervisor::HYPERVISOR_NAME_CH, CloudHypervisorConfig};

mod clh;

pub struct TestVm {
    pub hypervisor_name: String,
    pub hypervisor_instance: Arc<dyn Hypervisor>,
    pub socket_addr: String,
    pub is_hybrid_vsock: bool,
}

// Helper function to parse a configuration file.
// TO-DO: For now using a hard-coded temp path.
fn load_config() -> Result<TomlConfig> {
    const TMP_CONF_FILE: &str = "/tmp/configuration-clh.toml";

    debug!(sl!(), "test_vm_boot: Load kata configuration file");

    let (mut toml_config, _) = TomlConfig::load_from_file(TMP_CONF_FILE).context("test_vm_boot:Failed to load kata config file")?;

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

// virtiofsd - need to start this as well???
// Look into this for CLH
// crates/runtimes/virt_container/src/lib.rs: new_hypervisor()::CloudHypervisor::new()
//                                            set_config()
// crates/runtimes/virt_container/src/sandbox.rs:: start()
// Not sure but need to see what all happens with the hypervisor config we have from the toml
// Can we simply use that config and give it to CLH API client to create a vm??
// prepare_vm:
// - set ns to none
// - dont handle confidential guests
// - look at setting the run paths on host
//         // run_dir and vm_path are the same (shared)
//        self.run_dir = get_sandbox_path(&self.id);
//        self.vm_path = self.run_dir.to_string();
//
// Using the runtime-rs hypervisor crate.
// 1. Launch the clh process: Need to initialize CloudHypervisor::CloudHypervisorInner
// 2. Look into CloudHypervisorInner: start_hypervisor (are these pub functions??)
// 3. prepare_hypervisor
//    launch_hypervisor
//    prepare_vm
//    start_vm

// Helper method to boot a test pod VM
pub fn boot_test_vm() -> Result<TestVm> {
    debug!(sl!(), "boot_test_vm: Booting up a test pod vm...");

    // Register the hypervisor config plugin
    debug!(sl!(), "boot_test_vm: Register CLH plugin");
    let config = Arc::new(CloudHypervisorConfig::new());
    register_hypervisor_plugin(HYPERVISOR_NAME_CH, config);

    // get the kata configuration toml
    let toml_config = load_config()?;

    // determine the hypervisor
    let hypervisor_name = "cloud-hypervisor".to_string();
    let hypervisor_config = toml_config
        .hypervisor
        .get(&hypervisor_name)
        .ok_or_else(|| anyhow!("boot_test_vm: failed to get hypervisor for {}", &hypervisor_name))
        .context("get hypervisor name")?;

    // create a new hypervisor instance
    match hypervisor_name.as_str() {
        "cloud-hypervisor" => {
            return tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(clh::setup_test_vm(hypervisor_config.clone()))
                .context("pull and unpack container image");

        }
        "qemu" => {
            warn!(sl!(), "boot_test_vm: qemu is not implemented");
            return Err(anyhow!(
                "boot_test_vm: Hypervisor qemu is not implemented"
            ));
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
pub fn stop_test_vm(instance: Arc<dyn Hypervisor>) -> Result<()> {
    debug!(sl!(), "stop_test_vm: stopping booted vm");

    let _ = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(clh::stop_test_vm(instance))
        .context("stop booted test vm")?;

    Ok(())
}
