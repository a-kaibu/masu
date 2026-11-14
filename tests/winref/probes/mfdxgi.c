/* Media Foundation's Direct3D 11 buffers (#31): MFCreateDXGISurfaceBuffer on a B8G8R8A8 texture
 * (IMFDXGIBuffer: GetResource, GetSubresourceIndex, Get/SetUnknown; IMFMediaBuffer Lock reading
 * the texture, Unlock writing changes back; IMF2DBuffer pitch top-down and bottom-up; refused
 * arguments), then a Source Reader given the device (MF_SOURCE_READER_D3D_MANAGER, advanced
 * video processing) reading mfwmv's movie as RGB32 and as NV12: whether its samples are
 * textures, their size and format, and the colour of each frame through Lock; an NV12 texture's
 * luma and chroma views, cleared as render targets, boxes updated and copied; a device for video
 * given to a reader: the decoder transform, a sample allocator's textures. WARP on the Windows
 * runner.
 * libs: -lmfplat -lmfreadwrite -lmfuuid -ld3d11 -lole32 -luuid
 * run: browser (sake's D3D11 is WebGPU: tests/winref/run-browser.mjs) */
#define COBJMACROS
#include "../probe.h"
#include <initguid.h>
#include <d3d11.h>
#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <mferror.h>
#include "mfwmv_data.h"
#include "mfvideo_data.h"

DEFINE_GUID(KEY, 0x12345678, 0x1111, 0x2222, 0x33, 0x33, 0x44, 0x44, 0x55, 0x55, 0x66, 0x66);

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

/* the texel (x, y) of a texture, read back through a staging copy */
static DWORD texel(ID3D11Texture2D *t, int x, int y) {
    D3D11_TEXTURE2D_DESC d;
    ID3D11Texture2D_GetDesc(t, &d);
    d.Usage = D3D11_USAGE_STAGING;
    d.BindFlags = 0;
    d.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    d.MiscFlags = 0;
    d.MipLevels = d.ArraySize = 1;
    ID3D11Texture2D *s = NULL;
    if (FAILED(ID3D11Device_CreateTexture2D(dev, &d, NULL, &s))) return 0xdead;
    ID3D11DeviceContext_CopySubresourceRegion(ctx, (ID3D11Resource *)s, 0, 0, 0, 0, (ID3D11Resource *)t, 0, NULL);
    D3D11_MAPPED_SUBRESOURCE m;
    DWORD v = 0xbeef;
    if (SUCCEEDED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)s, 0, D3D11_MAP_READ, 0, &m))) {
        v = ((DWORD *)((BYTE *)m.pData + y * m.RowPitch))[x];
        ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)s, 0);
    }
    ID3D11Texture2D_Release(s);
    return v;
}

static void surface_buffer(void) {
    DWORD px[8 * 4];
    for (int i = 0; i < 32; i++) px[i] = 0xff000000 | (i * 0x070503);
    D3D11_TEXTURE2D_DESC d = { 8, 4, 1, 1, DXGI_FORMAT_B8G8R8A8_UNORM, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_SHADER_RESOURCE, 0, 0 };
    D3D11_SUBRESOURCE_DATA init = { px, 32, 0 };
    ID3D11Texture2D *t = NULL;
    HRESULT hr = ID3D11Device_CreateTexture2D(dev, &d, &init, &t);
    printf("texture: %#lx\n", hr);
    if (!t) return;
    IMFMediaBuffer *b = NULL;
    printf("bad riid: %#lx\n", MFCreateDXGISurfaceBuffer(&IID_IUnknown, (IUnknown *)t, 0, FALSE, &b));
    printf("subresource 1 of 1: %#lx\n", MFCreateDXGISurfaceBuffer(&IID_ID3D11Texture2D, (IUnknown *)t, 1, FALSE, &b));
    hr = MFCreateDXGISurfaceBuffer(&IID_ID3D11Texture2D, (IUnknown *)t, 0, FALSE, &b);
    printf("buffer: %#lx\n", hr);
    if (!b) return;
    DWORD max = 0, len = 0;
    IMFMediaBuffer_GetMaxLength(b, &max);
    IMFMediaBuffer_GetCurrentLength(b, &len);
    printf("  max %lu, current %lu\n", max, len);
    IMFDXGIBuffer *x = NULL;
    IMF2DBuffer *b2 = NULL;
    printf("  IMFDXGIBuffer %#lx, IMF2DBuffer %#lx\n", IMFMediaBuffer_QueryInterface(b, &IID_IMFDXGIBuffer, (void **)&x), IMFMediaBuffer_QueryInterface(b, &IID_IMF2DBuffer, (void **)&b2));
    if (x) {
        ID3D11Texture2D *r = NULL;
        UINT sub = 99;
        hr = IMFDXGIBuffer_GetResource(x, &IID_ID3D11Texture2D, (void **)&r);
        printf("  resource %#lx, the texture %d\n", hr, r == t);
        if (r) ID3D11Texture2D_Release(r);
        hr = IMFDXGIBuffer_GetSubresourceIndex(x, &sub);
        printf("  subresource %#lx %u\n", hr, sub);
        IUnknown *u = NULL;
        printf("  unknown not set: %#lx\n", IMFDXGIBuffer_GetUnknown(x, &KEY, &IID_IUnknown, (void **)&u));
        printf("  set: %#lx\n", IMFDXGIBuffer_SetUnknown(x, &KEY, (IUnknown *)t));
        printf("  set again: %#lx\n", IMFDXGIBuffer_SetUnknown(x, &KEY, (IUnknown *)t));
        hr = IMFDXGIBuffer_GetUnknown(x, &KEY, &IID_ID3D11Texture2D, (void **)&u);
        printf("  get: %#lx, the texture %d\n", hr, (void *)u == (void *)t);
        if (u) IUnknown_Release(u);
        printf("  set NULL: %#lx\n", IMFDXGIBuffer_SetUnknown(x, &KEY, NULL));
        u = NULL;
        printf("  get after: %#lx\n", IMFDXGIBuffer_GetUnknown(x, &KEY, &IID_IUnknown, (void **)&u));
    }
    BYTE *p = NULL;
    hr = IMFMediaBuffer_Lock(b, &p, &max, &len);
    printf("  lock %#lx, max %lu, current %lu\n", hr, max, len);
    if (p) {
        int same = 1;
        for (int i = 0; i < 32; i++)
            if (((DWORD *)p)[i] != px[i]) same = 0;
        printf("  the texture's bytes %d\n", same);
        ((DWORD *)p)[2 * 8 + 3] = 0xff123456;
        printf("  unlock %#lx\n", IMFMediaBuffer_Unlock(b));
        printf("  texel (3, 2) after: %#lx\n", texel(t, 3, 2));
    }
    printf("  unlock unlocked: %#lx\n", IMFMediaBuffer_Unlock(b));
    if (b2) {
        BYTE *s = NULL;
        LONG pitch = 0;
        BOOL cont = 2;
        DWORD clen = 0;
        printf("  scanline unlocked: %#lx\n", IMF2DBuffer_GetScanline0AndPitch(b2, &s, &pitch));
        hr = IMF2DBuffer_Lock2D(b2, &s, &pitch);
        printf("  lock2d %#lx, pitch %ld, row 1 texel 0 %#lx\n", hr, pitch, s ? *(DWORD *)(s + pitch) : 0);
        IMF2DBuffer_Unlock2D(b2);
        IMF2DBuffer_IsContiguousFormat(b2, &cont);
        IMF2DBuffer_GetContiguousLength(b2, &clen);
        printf("  contiguous %d, length %lu\n", cont, clen);
        IMF2DBuffer_Release(b2);
    }
    if (x) IMFDXGIBuffer_Release(x);
    IMFMediaBuffer_Release(b);
    b = NULL;
    MFCreateDXGISurfaceBuffer(&IID_ID3D11Texture2D, (IUnknown *)t, 0, TRUE, &b);
    if (b) {
        b2 = NULL;
        IMFMediaBuffer_QueryInterface(b, &IID_IMF2DBuffer, (void **)&b2);
        if (b2) {
            BYTE *s = NULL;
            LONG pitch = 0;
            hr = IMF2DBuffer_Lock2D(b2, &s, &pitch);
            printf("bottom-up lock2d %#lx, pitch %ld, scanline 0 texel 0 %#lx\n", hr, pitch, s ? *(DWORD *)s : 0);
            IMF2DBuffer_Unlock2D(b2);
            IMF2DBuffer_Release(b2);
        }
        IMFMediaBuffer_Release(b);
    }
    ID3D11Texture2D_Release(t);
}

static int classify(int y, int u, int v) {
    /* BT.601 studio range: red Y 81 U 90 V 240, lime 145 54 34, blue 41 240 110 */
    if (v > 180) return 'r';
    if (u > 180) return 'b';
    if (y > 100 && u < 100 && v < 100) return 'g';
    return '?';
}
static int classify_rgb(int r, int g, int b) {
    if (r > 180 && g < 80 && b < 80) return 'r';
    if (g > 180 && r < 80 && b < 80) return 'g';
    if (b > 180 && r < 80 && g < 80) return 'b';
    return '?';
}

static void nv12_views(void) {
    BYTE px[8 * 6];
    for (int i = 0; i < 48; i++) px[i] = i * 5;
    D3D11_TEXTURE2D_DESC d = { 8, 4, 1, 1, DXGI_FORMAT_NV12, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_SHADER_RESOURCE, 0, 0 };
    D3D11_SUBRESOURCE_DATA init = { px, 8, 0 };
    UINT sup = 0;
    HRESULT hr = ID3D11Device_CheckFormatSupport(dev, DXGI_FORMAT_NV12, &sup);
    printf("NV12 support: %#lx 2d %d sample %d lockable %d\n", hr, !!(sup & D3D11_FORMAT_SUPPORT_TEXTURE2D), !!(sup & D3D11_FORMAT_SUPPORT_SHADER_SAMPLE), !!(sup & D3D11_FORMAT_SUPPORT_CPU_LOCKABLE));
    ID3D11Texture2D *t = NULL;
    D3D11_TEXTURE2D_DESC odd = d;
    odd.Width = 7;
    printf("  odd width: %#lx\n", ID3D11Device_CreateTexture2D(dev, &odd, &init, &t));
    t = NULL;
    hr = ID3D11Device_CreateTexture2D(dev, &d, &init, &t);
    printf("  texture: %#lx\n", hr);
    if (!t) return;
    static const DXGI_FORMAT views[] = { DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8_UNORM, DXGI_FORMAT_NV12, DXGI_FORMAT_R8G8B8A8_UNORM };
    for (int i = 0; i < 4; i++) {
        D3D11_SHADER_RESOURCE_VIEW_DESC v = { views[i], D3D11_SRV_DIMENSION_TEXTURE2D };
        v.Texture2D.MipLevels = 1;
        ID3D11ShaderResourceView *srv = NULL;
        hr = ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)t, &v, &srv);
        printf("  view as %d: %#lx\n", views[i], hr);
        if (srv) ID3D11ShaderResourceView_Release(srv);
    }
    /* the bytes back through a staging copy: luma rows then chroma rows */
    d.Usage = D3D11_USAGE_STAGING;
    d.BindFlags = 0;
    d.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    ID3D11Texture2D *s = NULL;
    ID3D11Device_CreateTexture2D(dev, &d, NULL, &s);
    if (s) {
        ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)s, (ID3D11Resource *)t);
        D3D11_MAPPED_SUBRESOURCE m;
        if (SUCCEEDED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)s, 0, D3D11_MAP_READ, 0, &m))) {
            int same = 1;
            for (int y = 0; y < 6; y++)
                for (int x = 0; x < 8; x++)
                    if (((BYTE *)m.pData)[y * m.RowPitch + x] != px[y * 8 + x]) same = 0;
            printf("  staging copy: pitch>=8 %d, the bytes %d\n", m.RowPitch >= 8, same);
            ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)s, 0);
        }
        ID3D11Texture2D_Release(s);
    }
    ID3D11Texture2D_Release(t);
}

/* NV12 render targets: R8 (luma) and R8G8 (chroma) views cleared, an even box updated and one
   copied from another NV12 texture; the bytes back through a staging copy */
static void nv12_targets(void) {
    BYTE px[8 * 6], qx[8 * 6], box[4 * 3];
    for (int i = 0; i < 48; i++) px[i] = i * 5, qx[i] = 200 - i;
    for (int i = 0; i < 12; i++) box[i] = 0x80 + i;
    UINT sup = 0;
    ID3D11Device_CheckFormatSupport(dev, DXGI_FORMAT_NV12, &sup);
    printf("NV12 render target %d\n", !!(sup & D3D11_FORMAT_SUPPORT_RENDER_TARGET));
    D3D11_TEXTURE2D_DESC d = { 8, 4, 1, 1, DXGI_FORMAT_NV12, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET, 0, 0 };
    D3D11_SUBRESOURCE_DATA init = { px, 8, 0 }, init2 = { qx, 8, 0 };
    ID3D11Texture2D *t = NULL, *o = NULL, *s = NULL;
    printf("  texture: %#lx\n", ID3D11Device_CreateTexture2D(dev, &d, &init, &t));
    ID3D11Device_CreateTexture2D(dev, &d, &init2, &o);
    if (!t || !o) return;
    static const DXGI_FORMAT vf[] = { DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8_UNORM };
    ID3D11RenderTargetView *rtv[2] = { NULL, NULL };
    for (int i = 0; i < 2; i++) {
        D3D11_RENDER_TARGET_VIEW_DESC v = { vf[i], D3D11_RTV_DIMENSION_TEXTURE2D };
        printf("  render target view as %d: %#lx\n", vf[i], ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)t, &v, &rtv[i]));
    }
    if (rtv[0]) {
        float c[4] = { 0.5f, 0, 0, 0 };
        ID3D11DeviceContext_ClearRenderTargetView(ctx, rtv[0], c);
    }
    if (rtv[1]) {
        float c[4] = { 0.25f, 0.75f, 0, 0 };
        ID3D11DeviceContext_ClearRenderTargetView(ctx, rtv[1], c);
    }
    D3D11_BOX b = { 2, 0, 0, 6, 2, 1 };
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)t, 0, &b, box, 4, 0);
    D3D11_BOX cb = { 0, 2, 0, 2, 4, 1 };
    ID3D11DeviceContext_CopySubresourceRegion(ctx, (ID3D11Resource *)t, 0, 6, 2, 0, (ID3D11Resource *)o, 0, &cb);
    d.Usage = D3D11_USAGE_STAGING;
    d.BindFlags = 0;
    d.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    ID3D11Device_CreateTexture2D(dev, &d, NULL, &s);
    if (s) {
        ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)s, (ID3D11Resource *)t);
        D3D11_MAPPED_SUBRESOURCE m;
        if (SUCCEEDED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)s, 0, D3D11_MAP_READ, 0, &m))) {
            for (int y = 0; y < 6; y++) {
                printf("  row %d:", y);
                for (int x = 0; x < 8; x++) printf(" %02x", ((BYTE *)m.pData)[y * m.RowPitch + x]);
                printf("\n");
            }
            ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)s, 0);
        }
        ID3D11Texture2D_Release(s);
    }
    for (int i = 0; i < 2; i++)
        if (rtv[i]) ID3D11RenderTargetView_Release(rtv[i]);
    ID3D11Texture2D_Release(o);
    ID3D11Texture2D_Release(t);
}

static void reader(const WCHAR *path, const GUID *sub, const char *name) {
    UINT token = 0;
    IMFDXGIDeviceManager *m = NULL;
    HRESULT hr = MFCreateDXGIDeviceManager(&token, &m);
    printf("manager: %#lx\n", hr);
    if (!m) return;
    printf("  reset: %#lx\n", IMFDXGIDeviceManager_ResetDevice(m, (IUnknown *)dev, token));
    IMFAttributes *a = NULL;
    MFCreateAttributes(&a, 2);
    IMFAttributes_SetUnknown(a, &MF_SOURCE_READER_D3D_MANAGER, (IUnknown *)m);
    IMFAttributes_SetUINT32(a, &MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, TRUE);
    IMFSourceReader *r = NULL;
    hr = MFCreateSourceReaderFromURL(path, a, &r);
    IMFAttributes_Release(a);
    printf("reader with the device: %#lx\n", hr);
    if (r) {
        IMFSourceReader_SetStreamSelection(r, (DWORD)MF_SOURCE_READER_ALL_STREAMS, FALSE);
        IMFSourceReader_SetStreamSelection(r, MF_SOURCE_READER_FIRST_VIDEO_STREAM, TRUE);
        IMFMediaType *t;
        MFCreateMediaType(&t);
        IMFMediaType_SetGUID(t, &MF_MT_MAJOR_TYPE, &MFMediaType_Video);
        IMFMediaType_SetGUID(t, &MF_MT_SUBTYPE, sub);
        hr = IMFSourceReader_SetCurrentMediaType(r, MF_SOURCE_READER_FIRST_VIDEO_STREAM, NULL, t);
        IMFMediaType_Release(t);
        printf("  set %s: %#lx\n", name, hr);
        int nv12 = IsEqualGUID(sub, &MFVideoFormat_NV12);
        char cols[40] = { 0 }, gpu[40] = { 0 };
        int alpha_ok = 0;
        int n = 0, textures = 0, sized = 0;
        DXGI_FORMAT fmt = 0;
        for (int k = 0; k < 40; k++) {
            IMFSample *s = NULL;
            DWORD idx = 0, flags = 0;
            LONGLONG ts = 0;
            hr = IMFSourceReader_ReadSample(r, MF_SOURCE_READER_FIRST_VIDEO_STREAM, 0, &idx, &flags, &ts, &s);
            if (FAILED(hr)) {
                printf("  read %d failed %#lx\n", n, hr);
                break;
            }
            if (s) {
                IMFMediaBuffer *b = NULL;
                IMFSample_GetBufferByIndex(s, 0, &b);
                IMFDXGIBuffer *x = NULL;
                if (b && SUCCEEDED(IMFMediaBuffer_QueryInterface(b, &IID_IMFDXGIBuffer, (void **)&x))) {
                    ID3D11Texture2D *tx = NULL;
                    if (SUCCEEDED(IMFDXGIBuffer_GetResource(x, &IID_ID3D11Texture2D, (void **)&tx))) {
                        D3D11_TEXTURE2D_DESC d;
                        ID3D11Texture2D_GetDesc(tx, &d);
                        textures++;
                        if (d.Width == 64 && d.Height == 48) sized++;
                        fmt = d.Format;
                        /* the texture's own texels (what a shader draws), RGB ones */
                        if (!nv12 && n < 39) {
                            DWORD t = texel(tx, 32, 24);
                            gpu[n] = classify_rgb((t >> 16) & 255, (t >> 8) & 255, t & 255);
                            alpha_ok += (t >> 24) == 0xff;
                        }
                        ID3D11Texture2D_Release(tx);
                    }
                    IMFDXGIBuffer_Release(x);
                }
                BYTE *p = NULL;
                DWORD len = 0;
                int c = '?';
                if (b && SUCCEEDED(IMFMediaBuffer_Lock(b, &p, NULL, &len))) {
                    if (nv12 && len >= 64 * 48 * 3 / 2) {
                        const BYTE *uv = p + 64 * 48 + 12 * 64 + 32;
                        c = classify(p[24 * 64 + 32], uv[0], uv[1]);
                    } else if (!nv12 && len >= 64 * 48 * 4) {
                        const BYTE *q = p + (24 * 64 + 32) * 4;
                        c = classify_rgb(q[2], q[1], q[0]);
                    }
                    IMFMediaBuffer_Unlock(b);
                }
                if (n < 39) cols[n] = c;
                n++;
                if (b) IMFMediaBuffer_Release(b);
                IMFSample_Release(s);
            }
            if (flags & MF_SOURCE_READERF_ENDOFSTREAM) break;
        }
        printf("  %d frames %s, textures %d (64x48 %d, format %d)\n", n, cols, textures, sized, fmt);
        if (!nv12) printf("  texels %s, alpha 255 %d\n", gpu, alpha_ok);
        IMFSourceReader_Release(r);
    }
    IMFDXGIDeviceManager_Release(m);
}

/* A device for video (D3D11_CREATE_DEVICE_VIDEO_SUPPORT) given to a reader, as Unity's
   VideoPlayer does: the stream's decoder (GetServiceForStream(IID_IMFTransform)), its
   attributes and output type, and a sample allocator's samples (MFCreateVideoSampleAllocatorEx)
   as textures of the device */
static void decoder(const WCHAR *path) {
    D3D_FEATURE_LEVEL want = D3D_FEATURE_LEVEL_11_0, got;
    ID3D11Device *vd = NULL;
    HRESULT hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT, &want, 1, D3D11_SDK_VERSION, &vd, &got, NULL);
    if (FAILED(hr)) hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT, &want, 1, D3D11_SDK_VERSION, &vd, &got, NULL);
    printf("video device: %#lx\n", hr);
    if (!vd) return;
    ID3D10Multithread *mt = NULL;
    if (SUCCEEDED(ID3D11Device_QueryInterface(vd, &IID_ID3D10Multithread, (void **)&mt))) {
        printf("  protected: %d\n", ID3D10Multithread_SetMultithreadProtected(mt, TRUE) >= 0);
        ID3D10Multithread_Release(mt);
    }
    UINT token = 0;
    IMFDXGIDeviceManager *m = NULL;
    MFCreateDXGIDeviceManager(&token, &m);
    if (!m) return;
    printf("  reset: %#lx\n", IMFDXGIDeviceManager_ResetDevice(m, (IUnknown *)vd, token));
    IMFAttributes *a = NULL;
    MFCreateAttributes(&a, 3);
    IMFAttributes_SetUnknown(a, &MF_SOURCE_READER_D3D_MANAGER, (IUnknown *)m);
    IMFAttributes_SetUINT32(a, &MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, TRUE);
    IMFAttributes_SetUINT32(a, &MF_SA_D3D11_BINDFLAGS, D3D11_BIND_SHADER_RESOURCE);
    IMFSourceReader *r = NULL;
    hr = MFCreateSourceReaderFromURL(path, a, &r);
    IMFAttributes_Release(a);
    printf("  reader: %#lx\n", hr);
    if (r) {
        IMFMediaType *t = NULL;
        MFCreateMediaType(&t);
        IMFMediaType_SetGUID(t, &MF_MT_MAJOR_TYPE, &MFMediaType_Video);
        IMFMediaType_SetGUID(t, &MF_MT_SUBTYPE, &MFVideoFormat_NV12);
        printf("  set NV12: %#lx\n", IMFSourceReader_SetCurrentMediaType(r, MF_SOURCE_READER_FIRST_VIDEO_STREAM, NULL, t));
        IMFMediaType_Release(t);
        IMFTransform *x = NULL;
        printf("  audio decoder? %d\n", SUCCEEDED(IMFSourceReader_GetServiceForStream(r, MF_SOURCE_READER_MEDIASOURCE, &GUID_NULL, &IID_IMFTransform, (void **)&x)));
        if (x) IMFTransform_Release(x), x = NULL;
        hr = IMFSourceReader_GetServiceForStream(r, MF_SOURCE_READER_FIRST_VIDEO_STREAM, &GUID_NULL, &IID_IMFTransform, (void **)&x);
        printf("  decoder: %#lx\n", hr);
        if (x) {
            IMFAttributes *xa = NULL;
            if (SUCCEEDED(IMFTransform_GetAttributes(x, &xa)) && xa) {
                UINT32 aware = 0;
                IMFAttributes_GetUINT32(xa, &MF_SA_D3D11_AWARE, &aware);
                printf("  d3d11 aware: %u\n", aware);
                IMFAttributes_Release(xa);
            }
            IMFMediaType *o = NULL;
            if (SUCCEEDED(IMFTransform_GetOutputCurrentType(x, 0, &o)) && o) {
                UINT64 size = 0;
                GUID sub = { 0 };
                IMFMediaType_GetUINT64(o, &MF_MT_FRAME_SIZE, &size);
                IMFMediaType_GetGUID(o, &MF_MT_SUBTYPE, &sub);
                printf("  output %ux%u NV12 %d\n", (UINT)(size >> 32), (UINT)size, IsEqualGUID(&sub, &MFVideoFormat_NV12));
                IMFMediaType_Release(o);
            }
            IMFTransform_Release(x);
        }
        IMFSourceReader_Release(r);
    }
    /* the allocator of 64x48 ARGB32 frames, render targets and shader resources */
    IMFVideoSampleAllocatorEx *al = NULL;
    hr = MFCreateVideoSampleAllocatorEx(&IID_IMFVideoSampleAllocatorEx, (void **)&al);
    printf("allocator: %#lx\n", hr);
    if (al) {
        IMFSample *s = NULL;
        printf("  before init: %#lx\n", IMFVideoSampleAllocatorEx_AllocateSample(al, &s));
        printf("  manager: %#lx\n", IMFVideoSampleAllocatorEx_SetDirectXManager(al, (IUnknown *)m));
        IMFMediaType *t = NULL;
        MFCreateMediaType(&t);
        IMFMediaType_SetGUID(t, &MF_MT_MAJOR_TYPE, &MFMediaType_Video);
        IMFMediaType_SetGUID(t, &MF_MT_SUBTYPE, &MFVideoFormat_ARGB32);
        IMFMediaType_SetUINT64(t, &MF_MT_FRAME_SIZE, (UINT64)64 << 32 | 48);
        IMFAttributes *aa = NULL;
        MFCreateAttributes(&aa, 1);
        IMFAttributes_SetUINT32(aa, &MF_SA_D3D11_BINDFLAGS, D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE);
        printf("  init: %#lx\n", IMFVideoSampleAllocatorEx_InitializeSampleAllocatorEx(al, 0, 4, aa, t));
        IMFAttributes_Release(aa);
        IMFMediaType_Release(t);
        hr = IMFVideoSampleAllocatorEx_AllocateSample(al, &s);
        printf("  sample: %#lx\n", hr);
        if (s) {
            IMFMediaBuffer *b = NULL;
            IMFSample_GetBufferByIndex(s, 0, &b);
            IMFDXGIBuffer *x = NULL;
            if (b && SUCCEEDED(IMFMediaBuffer_QueryInterface(b, &IID_IMFDXGIBuffer, (void **)&x))) {
                ID3D11Texture2D *tx = NULL;
                if (SUCCEEDED(IMFDXGIBuffer_GetResource(x, &IID_ID3D11Texture2D, (void **)&tx))) {
                    D3D11_TEXTURE2D_DESC d;
                    ID3D11Texture2D_GetDesc(tx, &d);
                    ID3D11Device *td = NULL;
                    ID3D11Texture2D_GetDevice(tx, &td);
                    printf("  texture %ux%u format %d bind %#x usage %d, the video device %d\n", d.Width, d.Height, d.Format, d.BindFlags, d.Usage, td == vd);
                    if (td) ID3D11Device_Release(td);
                    ID3D11Texture2D_Release(tx);
                }
                IMFDXGIBuffer_Release(x);
            }
            if (b) IMFMediaBuffer_Release(b);
            IMFSample_Release(s);
        }
        IMFVideoSampleAllocatorEx_Release(al);
    }
    IMFDXGIDeviceManager_Release(m);
    ID3D11Device_Release(vd);
}

/* Unity's VideoPlayer with a device of its own (#227): the video device is not the program's,
   so its ARGB32 frames are opened on the program's device through their shared handles
   (IDXGIResource::GetSharedHandle, ID3D11Device::OpenSharedResource) and drawn from there */
static void shared(const WCHAR *path) {
    D3D_FEATURE_LEVEL want = D3D_FEATURE_LEVEL_11_0, got;
    ID3D11Device *vd = NULL;
    HRESULT hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT, &want, 1, D3D11_SDK_VERSION, &vd, &got, NULL);
    if (FAILED(hr)) hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT, &want, 1, D3D11_SDK_VERSION, &vd, &got, NULL);
    printf("shared: video device %#lx, another device %d\n", hr, vd && vd != dev);
    if (!vd) return;
    UINT token = 0;
    IMFDXGIDeviceManager *m = NULL;
    MFCreateDXGIDeviceManager(&token, &m);
    IMFDXGIDeviceManager_ResetDevice(m, (IUnknown *)vd, token);
    IMFAttributes *a = NULL;
    MFCreateAttributes(&a, 3);
    IMFAttributes_SetUnknown(a, &MF_SOURCE_READER_D3D_MANAGER, (IUnknown *)m);
    IMFAttributes_SetUINT32(a, &MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, TRUE);
    IMFAttributes_SetUINT32(a, &MF_SOURCE_READER_D3D11_BIND_FLAGS, D3D11_BIND_SHADER_RESOURCE);
    IMFSourceReader *r = NULL;
    hr = MFCreateSourceReaderFromURL(path, a, &r);
    IMFAttributes_Release(a);
    if (r) {
        IMFMediaType *t = NULL;
        MFCreateMediaType(&t);
        IMFMediaType_SetGUID(t, &MF_MT_MAJOR_TYPE, &MFMediaType_Video);
        IMFMediaType_SetGUID(t, &MF_MT_SUBTYPE, &MFVideoFormat_ARGB32);
        printf("  set ARGB32: %#lx\n", IMFSourceReader_SetCurrentMediaType(r, MF_SOURCE_READER_FIRST_VIDEO_STREAM, NULL, t));
        IMFMediaType_Release(t);
        char cols[40] = { 0 };
        int n = 0, handles = 0, opened = 0;
        for (int k = 0; k < 40 && n < 39; k++) {
            IMFSample *s = NULL;
            DWORD idx = 0, flags = 0;
            LONGLONG ts = 0;
            if (FAILED(IMFSourceReader_ReadSample(r, MF_SOURCE_READER_FIRST_VIDEO_STREAM, 0, &idx, &flags, &ts, &s))) break;
            if (s) {
                IMFMediaBuffer *b = NULL;
                IMFDXGIBuffer *x = NULL;
                ID3D11Texture2D *tx = NULL;
                char c = '-';
                IMFSample_GetBufferByIndex(s, 0, &b);
                if (b && SUCCEEDED(IMFMediaBuffer_QueryInterface(b, &IID_IMFDXGIBuffer, (void **)&x)) && SUCCEEDED(IMFDXGIBuffer_GetResource(x, &IID_ID3D11Texture2D, (void **)&tx))) {
                    IDXGIResource *res = NULL;
                    HANDLE h = NULL;
                    if (SUCCEEDED(ID3D11Texture2D_QueryInterface(tx, &IID_IDXGIResource, (void **)&res))) {
                        IDXGIResource_GetSharedHandle(res, &h);
                        IDXGIResource_Release(res);
                    }
                    handles += h != NULL;
                    IDXGIResource *o = NULL;
                    ID3D11Texture2D *ot = NULL;
                    if (h && SUCCEEDED(ID3D11Device_OpenSharedResource(dev, h, &IID_IDXGIResource, (void **)&o))) {
                        if (SUCCEEDED(IDXGIResource_QueryInterface(o, &IID_ID3D11Texture2D, (void **)&ot))) {
                            opened++;
                            DWORD v = texel(ot, 32, 24);
                            c = classify_rgb((v >> 16) & 255, (v >> 8) & 255, v & 255);
                            ID3D11Texture2D_Release(ot);
                        }
                        IDXGIResource_Release(o);
                    }
                    ID3D11Texture2D_Release(tx);
                }
                if (x) IMFDXGIBuffer_Release(x);
                if (b) IMFMediaBuffer_Release(b);
                IMFSample_Release(s);
                cols[n++] = c;
            }
            if (flags & MF_SOURCE_READERF_ENDOFSTREAM) break;
        }
        printf("  %d frames, handles %d, opened %d: %s\n", n, handles, opened, cols);
        IMFSourceReader_Release(r);
    }
    IMFDXGIDeviceManager_Release(m);
    ID3D11Device_Release(vd);
}

int main(void) {
    HRESULT hr = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    if (FAILED(hr)) {
        printf("skip no Media Foundation (%#lx)\n", hr);
        return 0;
    }
    D3D_FEATURE_LEVEL want = D3D_FEATURE_LEVEL_11_0, got;
    hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, D3D11_CREATE_DEVICE_BGRA_SUPPORT, &want, 1, D3D11_SDK_VERSION, &dev, &got, &ctx);
    if (FAILED(hr)) hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, D3D11_CREATE_DEVICE_BGRA_SUPPORT, &want, 1, D3D11_SDK_VERSION, &dev, &got, &ctx);
    if (FAILED(hr)) {
        printf("skip D3D11CreateDevice %#lx\n", hr);
        return 0;
    }
    ID3D10Multithread *mt = NULL;
    if (SUCCEEDED(ID3D11Device_QueryInterface(dev, &IID_ID3D10Multithread, (void **)&mt))) {
        ID3D10Multithread_SetMultithreadProtected(mt, TRUE);
        ID3D10Multithread_Release(mt);
    }
    surface_buffer();
    WCHAR dir[MAX_PATH], path[MAX_PATH];
    GetTempPathW(MAX_PATH, dir);
    swprintf(path, MAX_PATH, L"%lsmfdxgi-probe.wmv", dir);
    HANDLE f = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
    DWORD w;
    WriteFile(f, file_wmv, sizeof file_wmv, &w, NULL);
    CloseHandle(f);
    nv12_views();
    nv12_targets();
    reader(path, &MFVideoFormat_RGB32, "RGB32");
    reader(path, &MFVideoFormat_NV12, "NV12");
    reader(path, &MFVideoFormat_ARGB32, "ARGB32");
    decoder(path);
    DeleteFileW(path);
    /* mfvideo's H.264 movie (the same colours): the frames the page decodes, as textures */
    swprintf(path, MAX_PATH, L"%lsmfdxgi-probe.mp4", dir);
    f = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
    WriteFile(f, file_mp4, sizeof file_mp4, &w, NULL);
    CloseHandle(f);
    reader(path, &MFVideoFormat_ARGB32, "ARGB32 of H.264");
    reader(path, &MFVideoFormat_NV12, "NV12 of H.264");
    shared(path);
    DeleteFileW(path);
    ID3D11DeviceContext_Release(ctx);
    ID3D11Device_Release(dev);
    MFShutdown();
    return 0;
}
