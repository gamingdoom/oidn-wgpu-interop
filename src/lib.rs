pub mod dx12;
pub mod vulkan;

#[derive(Eq, PartialEq, Clone, Copy, Debug)]
enum Backend {
    Dx12,
    Vulkan,
}

pub struct Device {
    wgpu_device: wgpu::Device,
    oidn_device: oidn::Device,
    queue: wgpu::Queue,
    backend: Backend,
}

impl Device {
    pub async fn new(
        adapter: &wgpu::Adapter,
        desc: &wgpu::DeviceDescriptor<'_>,
        trace_path: Option<&std::path::Path>,
    ) -> Result<(Self, wgpu::Queue), Option<wgpu::RequestDeviceError>> {
        match adapter.get_info().backend {
            wgpu::Backend::Vulkan => {
                Self::new_vulkan(adapter, desc, trace_path).await
            }
            wgpu::Backend::Dx12 => {
                Self::new_dx12(adapter, desc, trace_path).await
            }
            _ => Err(None),
        }
    }
    pub fn allocate_shared_buffers(&self, size: wgpu::BufferAddress) -> Result<SharedBuffer, Option<()>> {
        match self.backend {
            Backend::Dx12 => {
                self.allocate_shared_buffers_dx12(size)
            }
            Backend::Vulkan => {
                self.allocate_shared_buffers_vulkan(size)
            }
        }
    }
    pub fn oidn_device(&self) -> &oidn::Device {
        &self.oidn_device
    }

    pub fn wgpu_device(&self) -> &wgpu::Device {
        &self.wgpu_device
    }
}

enum Allocation {
    // we keep these around to keep the allocations alive
    Dx12 { _dx12: dx12::Dx12Allocation },
    Vulkan { _vulkan: vulkan::VulkanAllocation },
}

pub struct SharedBuffer {
    _allocation: Allocation,
    oidn_buffer: oidn::Buffer,
    wgpu_buffer: wgpu::Buffer,
}

impl SharedBuffer {
    pub fn oidn_buffer(&self) -> &oidn::Buffer {
        &self.oidn_buffer
    }
    pub fn oidn_buffer_mut(&mut self) -> &mut oidn::Buffer {
        &mut self.oidn_buffer
    }
    pub fn wgpu_buffer(&self) -> &wgpu::Buffer {
        &self.wgpu_buffer
    }
}

#[cfg(test)]
#[async_std::test]
async fn test() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12 | wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapters = instance
        .enumerate_adapters(wgpu::Backends::all());
    for adapter in adapters {
        match adapter.get_info().backend {
            wgpu::Backend::Vulkan => {
                eprintln!("Testing vulkan device {}", adapter.get_info().name);
            }
            wgpu::Backend::Dx12 => {
                eprintln!("Testing dx12 device {}", adapter.get_info().name);
            }
            _ => continue,
        }
        let Ok((device, queue)) = Device::new(&adapter, &wgpu::DeviceDescriptor::default(), None)
            .await else {
            eprintln!("Failed to create device {} (is oidn not supported on it?)", adapter.get_info().name);
            continue;
        };
        let mut bufs = device
            .allocate_shared_buffers(size_of::<[f32; 4]>() as wgpu::BufferAddress)
            .unwrap();
        queue.write_buffer(bufs.wgpu_buffer(), 0, &1.0_f32.to_ne_bytes());
        queue.submit([]);
        device
            .wgpu_device()
            .poll(wgpu::Maintain::Wait)
            .panic_on_timeout();
        assert_eq!(bufs.oidn_buffer_mut().read()[0], 1.0);
    }
}
