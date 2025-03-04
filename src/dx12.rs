use oidn::sys::OIDNExternalMemoryTypeFlag_OIDN_EXTERNAL_MEMORY_TYPE_FLAG_OPAQUE_WIN32;
use std::ptr;
use wgpu::hal::api::Dx12;
use wgpu::hal::{CommandEncoder, dx12};
use wgpu::{BufferDescriptor, BufferUsages, DeviceDescriptor, RequestDeviceError};
use windows::Win32::Foundation::GENERIC_ALL;
use windows::Win32::Graphics::Direct3D12::{
    D3D12_CPU_PAGE_PROPERTY_NOT_AVAILABLE, D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT,
    D3D12_HEAP_DESC, D3D12_HEAP_FLAG_SHARED, D3D12_HEAP_FLAG_SHARED_CROSS_ADAPTER,
    D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_CUSTOM, D3D12_MEMORY_POOL_L0, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_BUFFER, D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER,
    D3D12_RESOURCE_STATE_COMMON, D3D12_TEXTURE_LAYOUT_ROW_MAJOR, ID3D12Heap,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC};

pub struct DX12Device {
    wgpu_device: wgpu::Device,
    oidn_device: oidn::Device,
    queue: wgpu::Queue,
}

pub struct Dx12Buffer {
    _heap: ID3D12Heap,
    oidn_buffer: oidn::Buffer,
    wgpu_buffer: wgpu::Buffer,
}

impl DX12Device {
    pub async fn new(
        adapter: &wgpu::Adapter,
        desc: &DeviceDescriptor<'_>,
        trace_path: Option<&std::path::Path>,
    ) -> Result<(Self, wgpu::Queue), Option<RequestDeviceError>> {
        // # SAFETY: the raw handle is not manually destroyed.
        let adapter_dx12_desc = unsafe {
            adapter.as_hal::<Dx12, _, _>(|adapter| {
                adapter.map(|adapter| adapter.raw_adapter().GetDesc2().unwrap())
            })
        };
        let Some(dx_desc) = adapter_dx12_desc else {
            return Err(None);
        };
        let device = unsafe {
            oidn::sys::oidnNewDeviceByLUID((&dx_desc.AdapterLuid) as *const _ as *const _)
        };
        if device.is_null() {
            return Err(None);
        }
        let (wgpu_device, queue) = adapter.request_device(desc, trace_path).await?;
        let oidn_device = unsafe {
            oidn::sys::oidnCommitDevice(device);
            oidn::Device::from_raw(device)
        };
        Ok((
            DX12Device {
                wgpu_device,
                oidn_device,
                queue: queue.clone(),
            },
            queue,
        ))
    }
    pub fn allocate_buffers(
        &self,
        size: wgpu::BufferAddress,
        count: u8,
    ) -> Result<Vec<Dx12Buffer>, Option<()>> {
        let mut buffers = Vec::with_capacity(count as usize);
        if size == 0 || count == 0 {
            return Err(None);
        }
        unsafe {
            self.wgpu_device.as_hal::<Dx12, _, _>(|device| {
                let device = device.unwrap();
                for i in 0..count {
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
                            eprintln!(
                                "Failed to create resource number {}: {}",
                                i as u16 + 1,
                                err.message()
                            );
                            None
                        })?;
                    // it should really be this, but for some reason it doesn't work and the other
                    // way works fine
                    /*device
                    .raw_device()
                    .CreateCommittedResource(
                        &properties,
                        flags,
                        &desc,
                        D3D12_RESOURCE_STATE_COMMON,
                        None,
                        &mut resource,
                    )
                    .map_err(|err| {
                        eprintln!("Failed to create resource number {}: {}", i as u16 + 1, err.message());
                        None
                    })?;*/
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
                        eprintln!("Failed to create oidn buffer number {}", i + 1);
                        eprintln!("error: {:?}", self.oidn_device.get_error());
                        return Err(None);
                    }
                    let buf = dx12::Device::buffer_from_raw(resource, size);
                    let mut encoder = self.wgpu_device.create_command_encoder(&Default::default());
                    encoder.as_hal_mut::<Dx12, _, _>(|encoder| {
                        encoder.unwrap().clear_buffer(&buf, 0..size);
                    });
                    self.queue.submit([encoder.finish()]);
                    let wgpu_buffer = self.wgpu_device.create_buffer_from_hal::<Dx12>(
                        buf,
                        &BufferDescriptor {
                            label: None,
                            size,
                            usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        },
                    );
                    buffers.push(Dx12Buffer {
                        _heap: heap,
                        wgpu_buffer,
                        oidn_buffer: self.oidn_device.create_buffer_from_raw(oidn_buffer),
                    })
                }
                Ok(())
            })?
        }
        Ok(buffers)
    }
    pub fn oidn_device(&self) -> &oidn::Device {
        &self.oidn_device
    }

    pub fn wgpu_device(&self) -> &wgpu::Device {
        &self.wgpu_device
    }
}

impl Dx12Buffer {
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
