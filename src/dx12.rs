use oidn::sys::OIDNExternalMemoryTypeFlag_OIDN_EXTERNAL_MEMORY_TYPE_FLAG_OPAQUE_WIN32;
use std::ptr;
use wgpu::hal::api::Dx12;
use wgpu::hal::{CommandEncoder, dx12};
use wgpu::{BufferDescriptor, BufferUsages, DeviceDescriptor};
use windows::Win32::Foundation::GENERIC_ALL;
use windows::Win32::Graphics::Direct3D12::{
    D3D12_CPU_PAGE_PROPERTY_NOT_AVAILABLE, D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT,
    D3D12_HEAP_DESC, D3D12_HEAP_FLAG_SHARED, D3D12_HEAP_FLAG_SHARED_CROSS_ADAPTER,
    D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_CUSTOM, D3D12_MEMORY_POOL_L0, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_BUFFER, D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER,
    D3D12_RESOURCE_STATE_COMMON, D3D12_TEXTURE_LAYOUT_ROW_MAJOR, ID3D12Heap,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC};

pub(crate) struct Dx12Allocation {
    _heap: ID3D12Heap,
}

impl crate::Device {
    pub(crate) async fn new_dx12(
        adapter: &wgpu::Adapter,
        desc: &DeviceDescriptor<'_>,
        trace_path: Option<&std::path::Path>,
    ) -> Result<(Self, wgpu::Queue), crate::DeviceCreateError> {
        // # SAFETY: the raw handle is not manually destroyed.
        let adapter_dx12_desc = unsafe {
            adapter.as_hal::<Dx12, _, _>(|adapter| {
                adapter.map(|adapter| adapter.raw_adapter().GetDesc2().unwrap())
            })
        };
        let Some(dx_desc) = adapter_dx12_desc else {
            return Err(crate::DeviceCreateError::UnsupportedBackend(
                adapter.get_info().backend,
            ));
        };
        let device = unsafe {
            oidn::sys::oidnNewDeviceByLUID((&dx_desc.AdapterLuid) as *const _ as *const _)
        };
        if device.is_null() {
            return Err(crate::DeviceCreateError::OidnUnsupported);
        }
        let (wgpu_device, queue) = adapter
            .request_device(desc, trace_path)
            .await
            .map_err(|err| crate::DeviceCreateError::RequestDeviceError(err))?;
        let oidn_device = unsafe {
            oidn::sys::oidnCommitDevice(device);
            oidn::Device::from_raw(device)
        };
        Ok((
            crate::Device {
                wgpu_device,
                oidn_device,
                queue: queue.clone(),
                backend: crate::Backend::Dx12,
            },
            queue,
        ))
    }
    pub(crate) fn allocate_shared_buffers_dx12(
        &self,
        size: wgpu::BufferAddress,
    ) -> Result<crate::SharedBuffer, Option<()>> {
        assert_eq!(self.backend, crate::Backend::Dx12);

        if size == 0 {
            return Err(None);
        }
        // # SAFETY: the raw handle is not manually destroyed.
        unsafe {
            self.wgpu_device.as_hal::<Dx12, _, _>(|device| {
                let device = device.unwrap();
                let properties = D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_CUSTOM,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_NOT_AVAILABLE,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_L0,
                    CreationNodeMask: 0,
                    VisibleNodeMask: 0,
                };
                let flags = D3D12_HEAP_FLAG_SHARED_CROSS_ADAPTER | D3D12_HEAP_FLAG_SHARED;
                let heap_desc = D3D12_HEAP_DESC {
                    SizeInBytes: size,
                    Properties: properties,
                    Alignment: D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT as u64,
                    Flags: flags,
                };
                let mut heap = None;
                // Note on safety, since we keep the heap separate from the buffer even if
                // the buffer is destroyed we don't destroy the backing memory, which allows the
                // oidn buffer to function as usual
                device
                    .raw_device()
                    .CreateHeap(&heap_desc, &mut heap)
                    .map_err(|err| {
                        eprintln!("Failed to create heap: {}", err.message());
                        None
                    })?;
                let heap: ID3D12Heap = heap.unwrap();
                let desc = D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Alignment: 0,
                    Width: size,
                    Height: 1,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_UNKNOWN,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                    Flags: D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER,
                };
                let mut resource = None;
                device
                    .raw_device()
                    .CreatePlacedResource(
                        &heap,
                        0,
                        &desc,
                        D3D12_RESOURCE_STATE_COMMON,
                        None,
                        &mut resource,
                    )
                    .map_err(|err| {
                        eprintln!("Failed to create resource: {}", err.message());
                        None
                    })?;
                let resource = resource.unwrap();
                let handle = device
                    .raw_device()
                    .CreateSharedHandle(&heap, None, GENERIC_ALL.0, None)
                    .map_err(|err| {
                        eprintln!("Failed to create shared handle: {}", err.message());
                        None
                    })?;
                let oidn_buffer = oidn::sys::oidnNewSharedBufferFromWin32Handle(
                    self.oidn_device.raw(),
                    OIDNExternalMemoryTypeFlag_OIDN_EXTERNAL_MEMORY_TYPE_FLAG_OPAQUE_WIN32,
                    handle.0,
                    ptr::null(),
                    size as usize,
                );
                if oidn_buffer.is_null() {
                    eprintln!("Failed to create oidn buffer");
                    eprintln!("error: {:?}", self.oidn_device.get_error());
                    return Err(None);
                }
                let buf = dx12::Device::buffer_from_raw(resource, size);
                // # SAFETY: the raw handle is not manually destroyed.
                let mut encoder = self.wgpu_device.create_command_encoder(&Default::default());
                encoder.as_hal_mut::<Dx12, _, _>(|encoder| {
                    encoder.unwrap().clear_buffer(&buf, 0..size);
                });
                self.queue.submit([encoder.finish()]);
                // # SAFETY: Just initialized buffer, created it from the same device and made with
                // the manually mapped usages.
                let wgpu_buffer = self.wgpu_device.create_buffer_from_hal::<Dx12>(
                    buf,
                    &BufferDescriptor {
                        label: None,
                        size,
                        usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    },
                );
                Ok(crate::SharedBuffer {
                    _allocation: crate::Allocation::Dx12 {
                        _dx12: Dx12Allocation { _heap: heap },
                    },
                    wgpu_buffer,
                    oidn_buffer: self.oidn_device.create_buffer_from_raw(oidn_buffer),
                })
            })
        }
    }
}
