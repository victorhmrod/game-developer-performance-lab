use gdpl_core::Hardware;
use sysinfo::System;

pub fn inspect() -> Hardware {
    let mut system = System::new_all();
    system.refresh_cpu_all();
    Hardware {
        os: System::name(),
        os_version: System::os_version(),
        cpu: system.cpus().first().map(|cpu| cpu.brand().to_string()),
        logical_cpus: system.cpus().len(),
        total_memory_bytes: system.total_memory(),
    }
}
