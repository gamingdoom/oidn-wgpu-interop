pub mod dx12;

#[async_std::test]
async fn dx12() {
    use dx12::*;
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..Default::default()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .unwrap();
    let (device, queue) = DX12Device::new(&adapter, &wgpu::DeviceDescriptor::default(), None)
        .await
        .unwrap();
    let mut bufs = device.allocate_buffers(size_of::<[f32; 4]>() as wgpu::BufferAddress, 3).unwrap();
    queue.write_buffer(bufs[0].wgpu_buffer(), 0, &1.0_f32.to_ne_bytes());
    queue.submit([]);
    device.wgpu_device().poll(wgpu::Maintain::Wait).panic_on_timeout();
    assert_eq!(bufs[0].oidn_buffer_mut().read()[0], 1.0);
}
