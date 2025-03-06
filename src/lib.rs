use std::fmt::Debug;

pub mod dx12;
pub mod vulkan;

pub enum DeviceCreateError {
    RequestDeviceError(wgpu::RequestDeviceError),
    OidnUnsupported,
    OidnImportUnsupported,
    MissingFeature,
    UnsupportedBackend(wgpu::Backend),
}

impl Debug for DeviceCreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DeviceCreateError::RequestDeviceError(err) => err.fmt(f),
            DeviceCreateError::OidnUnsupported => f.write_str(
                "OIDN could not create a device for this Adapter (does this adapter support OIDN?)",
            ),
            DeviceCreateError::OidnImportUnsupported => {
                f.write_str("OIDN does not support the required import method")
            }
            DeviceCreateError::MissingFeature => f.write_str("A required feature is missing"),
            DeviceCreateError::UnsupportedBackend(backend) => {
                f.write_str("The backend ")?;
                backend.fmt(f)?;
                f.write_str(" is not supported.")
            }
        }
    }
}

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
    ) -> Result<(Self, wgpu::Queue), DeviceCreateError> {
        match adapter.get_info().backend {
            wgpu::Backend::Vulkan => Self::new_vulkan(adapter, desc, trace_path).await,
            wgpu::Backend::Dx12 => Self::new_dx12(adapter, desc, trace_path).await,
            _ => Err(DeviceCreateError::UnsupportedBackend(
                adapter.get_info().backend,
            )),
        }
    }
    pub fn allocate_shared_buffers(
        &self,
        size: wgpu::BufferAddress,
    ) -> Result<SharedBuffer, Option<()>> {
        match self.backend {
            Backend::Dx12 => self.allocate_shared_buffers_dx12(size),
            Backend::Vulkan => self.allocate_shared_buffers_vulkan(size),
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
    let adapters = instance.enumerate_adapters(wgpu::Backends::all());
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
        let (device, queue) =
            match Device::new(&adapter, &wgpu::DeviceDescriptor::default(), None).await {
                Ok((device, queue)) => (device, queue),
                Err(err) => {
                    eprintln!("Device creation failed");
                    eprintln!("    {err:?}");
                    continue;
                }
            };
        let mut bufs = device
            .allocate_shared_buffers(size_of::<[f32; 3]>() as wgpu::BufferAddress)
            .unwrap();
        queue.write_buffer(bufs.wgpu_buffer(), 0, &1.0_f32.to_ne_bytes());
        queue.submit([]);
        device
            .wgpu_device()
            .poll(wgpu::Maintain::Wait)
            .panic_on_timeout();
        assert_eq!(bufs.oidn_buffer_mut().read()[0], 1.0);
        let mut filter = oidn::RayTracing::new(device.oidn_device());
        filter.image_dimensions(1, 1);
        filter
            .filter_in_place_buffer(&mut bufs.oidn_buffer_mut())
            .unwrap();
    }
}
