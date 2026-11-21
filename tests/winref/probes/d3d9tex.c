/* Direct3D 9 texture bookkeeping: SetPriority/GetPriority and SetLOD/GetLOD of each pool
 * (textures, their surfaces, vertex buffers); D3DUSAGE_AUTOGENMIPMAP (CheckDeviceFormat's
 * answer, CreateTexture's level counts and refusals, Set/GetAutoGenFilterType, the levels it
 * makes: a 4x4 texture of four colored quadrants sampled at its 2x2 and 1x1 levels through
 * D3DSAMP_MAXMIPLEVEL); dirty regions (a system memory texture locked with and without
 * D3DLOCK_NO_DIRTY_UPDATE, AddDirtyRect, then UpdateTexture into a default pool one, read
 * back by drawing it).
 * libs: -ld3d9
 * arch: 32
 * run: browser (sake draws through WebGPU: tests/winref/run-browser.mjs) */
#define COBJMACROS
#include "../probe.h"
#include <d3d9.h>

#define W 8
#define H 8

static IDirect3DDevice9 *dev;
static IDirect3DSurface9 *back, *bsys;

struct tv { float x, y, z, rhw; float u, v; };

/* the whole back buffer drawn from `t` (point sampled, its level `mip` and down) */
static void quad(IDirect3DBaseTexture9 *t, DWORD mip) {
    struct tv v[4] = { { -0.5f, -0.5f, 0, 1, 0, 0 }, { W - 0.5f, -0.5f, 0, 1, 1, 0 }, { -0.5f, H - 0.5f, 0, 1, 0, 1 }, { W - 0.5f, H - 0.5f, 0, 1, 1, 1 } };
    IDirect3DDevice9_Clear(dev, 0, NULL, D3DCLEAR_TARGET, 0xff102030, 1, 0);
    IDirect3DDevice9_SetTexture(dev, 0, t);
    IDirect3DDevice9_SetSamplerState(dev, 0, D3DSAMP_MINFILTER, D3DTEXF_POINT);
    IDirect3DDevice9_SetSamplerState(dev, 0, D3DSAMP_MAGFILTER, D3DTEXF_POINT);
    IDirect3DDevice9_SetSamplerState(dev, 0, D3DSAMP_MIPFILTER, D3DTEXF_POINT);
    IDirect3DDevice9_SetSamplerState(dev, 0, D3DSAMP_MAXMIPLEVEL, mip);
    IDirect3DDevice9_SetTextureStageState(dev, 0, D3DTSS_COLOROP, D3DTOP_SELECTARG1);
    IDirect3DDevice9_SetTextureStageState(dev, 0, D3DTSS_COLORARG1, D3DTA_TEXTURE);
    IDirect3DDevice9_SetFVF(dev, D3DFVF_XYZRHW | D3DFVF_TEX1);
    IDirect3DDevice9_BeginScene(dev);
    IDirect3DDevice9_DrawPrimitiveUP(dev, D3DPT_TRIANGLESTRIP, 2, v, sizeof v[0]);
    IDirect3DDevice9_EndScene(dev);
    IDirect3DDevice9_SetTexture(dev, 0, NULL);
}

/* the back buffer's pixels (1, 1), (6, 1), (1, 6), (6, 6), without alpha */
static void out(const char *what) {
    D3DLOCKED_RECT lr;
    printf("  %s", what);
    if (FAILED(IDirect3DDevice9_GetRenderTargetData(dev, back, bsys)) || FAILED(IDirect3DSurface9_LockRect(bsys, &lr, NULL, D3DLOCK_READONLY))) {
        printf(" readback failed\n");
        return;
    }
    static const int at[4][2] = { { 1, 1 }, { 6, 1 }, { 1, 6 }, { 6, 6 } };
    for (int k = 0; k < 4; k++) printf(" %06lx", ((DWORD *)((char *)lr.pBits + at[k][1] * lr.Pitch))[at[k][0]] & 0xffffff);
    IDirect3DSurface9_UnlockRect(bsys);
    printf("\n");
}

/* the 4x4 level 0 of `t`: quadrants red, green, blue, white (rect NULL) or one color in `r` */
static void fill(IDirect3DTexture9 *t, const RECT *r, DWORD flags, DWORD c) {
    D3DLOCKED_RECT lr;
    if (FAILED(IDirect3DTexture9_LockRect(t, 0, &lr, r, flags))) { printf("  LockRect failed\n"); return; }
    static const DWORD q[4] = { 0xffff0000, 0xff00ff00, 0xff0000ff, 0xffffffff };
    int w = r ? r->right - r->left : 4, h = r ? r->bottom - r->top : 4;
    for (int y = 0; y < h; y++)
        for (int x = 0; x < w; x++) ((DWORD *)((char *)lr.pBits + y * lr.Pitch))[x] = r ? c : q[(y >= 2) * 2 + (x >= 2)];
    IDirect3DTexture9_UnlockRect(t, 0);
}

static const char *pools[4] = { "DEFAULT", "MANAGED", "SYSTEMMEM", "SCRATCH" };

int main(void) {
    WNDCLASSA wc = { 0, DefWindowProcA, 0, 0, GetModuleHandleA(NULL), 0, 0, 0, 0, "probe" };
    RegisterClassA(&wc);
    HWND hwnd = CreateWindowA("probe", "d3d9tex", WS_POPUP, 0, 0, W, H, 0, 0, wc.hInstance, 0);
    IDirect3D9 *d3d = Direct3DCreate9(D3D_SDK_VERSION);
    if (!d3d) { puts("skip no Direct3D 9"); return 0; }
    D3DPRESENT_PARAMETERS pp = { 0 };
    pp.BackBufferWidth = W; pp.BackBufferHeight = H; pp.BackBufferFormat = D3DFMT_X8R8G8B8; pp.BackBufferCount = 1;
    pp.SwapEffect = D3DSWAPEFFECT_DISCARD; pp.hDeviceWindow = hwnd; pp.Windowed = TRUE;
    HRESULT hr = IDirect3D9_CreateDevice(d3d, D3DADAPTER_DEFAULT, D3DDEVTYPE_HAL, hwnd, D3DCREATE_SOFTWARE_VERTEXPROCESSING, &pp, &dev);
    if (FAILED(hr)) { printf("skip CreateDevice %#lx\n", hr); return 0; }
    IDirect3DDevice9_GetBackBuffer(dev, 0, 0, D3DBACKBUFFER_TYPE_MONO, &back);
    IDirect3DDevice9_CreateOffscreenPlainSurface(dev, W, H, D3DFMT_X8R8G8B8, D3DPOOL_SYSTEMMEM, &bsys, NULL);
    IDirect3DDevice9_SetRenderState(dev, D3DRS_LIGHTING, FALSE);
    IDirect3DDevice9_SetRenderState(dev, D3DRS_CULLMODE, D3DCULL_NONE);

    puts("priority and LOD of each pool (texture: Get, Set(1), Get, Set(2); surface: Get, Set(1), Get; SetLOD(1), SetLOD(5), GetLOD)");
    for (int p = 0; p < 4; p++) {
        IDirect3DTexture9 *t = NULL;
        hr = IDirect3DDevice9_CreateTexture(dev, 16, 16, 3, 0, D3DFMT_A8R8G8B8, (D3DPOOL)p, &t, NULL);
        printf("  %s CreateTexture %#lx", pools[p], hr);
        if (SUCCEEDED(hr)) {
            DWORD a = IDirect3DTexture9_GetPriority(t), b = IDirect3DTexture9_SetPriority(t, 1), c = IDirect3DTexture9_GetPriority(t), e = IDirect3DTexture9_SetPriority(t, 2);
            printf(" texture %lu %lu %lu %lu", a, b, c, e);
            IDirect3DSurface9 *s = NULL;
            IDirect3DTexture9_GetSurfaceLevel(t, 0, &s);
            a = IDirect3DSurface9_GetPriority(s), b = IDirect3DSurface9_SetPriority(s, 1), c = IDirect3DSurface9_GetPriority(s);
            printf(" surface %lu %lu %lu", a, b, c);
            IDirect3DSurface9_Release(s);
            a = IDirect3DTexture9_SetLOD(t, 1), b = IDirect3DTexture9_SetLOD(t, 5), c = IDirect3DTexture9_GetLOD(t);
            printf(" lod %lu %lu %lu", a, b, c);
            IDirect3DTexture9_Release(t);
        }
        IDirect3DVertexBuffer9 *vb = NULL;
        hr = IDirect3DDevice9_CreateVertexBuffer(dev, 64, 0, 0, (D3DPOOL)p, &vb, NULL);
        printf(" vb %#lx", hr);
        if (SUCCEEDED(hr)) {
            DWORD a = IDirect3DVertexBuffer9_GetPriority(vb), b = IDirect3DVertexBuffer9_SetPriority(vb, 3), c = IDirect3DVertexBuffer9_GetPriority(vb);
            printf(" %lu %lu %lu", a, b, c);
            IDirect3DVertexBuffer9_Release(vb);
        }
        printf("\n");
    }

    puts("CheckDeviceFormat D3DUSAGE_AUTOGENMIPMAP (texture; dynamic too)");
    static const struct { D3DFORMAT f; const char *n; } fm[] = {
        { D3DFMT_A8R8G8B8, "A8R8G8B8" }, { D3DFMT_X8R8G8B8, "X8R8G8B8" }, { D3DFMT_R5G6B5, "R5G6B5" }, { D3DFMT_A8, "A8" },
        { D3DFMT_L8, "L8" }, { D3DFMT_A8L8, "A8L8" }, { D3DFMT_V8U8, "V8U8" }, { D3DFMT_DXT1, "DXT1" }, { D3DFMT_A16B16G16R16F, "A16B16G16R16F" },
        { D3DFMT_R32F, "R32F" }, { D3DFMT_D16, "D16" },
        { D3DFMT_A8B8G8R8, "A8B8G8R8" }, { D3DFMT_X8B8G8R8, "X8B8G8R8" }, { D3DFMT_X1R5G5B5, "X1R5G5B5" }, { D3DFMT_A1R5G5B5, "A1R5G5B5" },
        { D3DFMT_A4R4G4B4, "A4R4G4B4" }, { D3DFMT_A2R10G10B10, "A2R10G10B10" }, { D3DFMT_A2B10G10R10, "A2B10G10R10" }, { D3DFMT_G16R16, "G16R16" },
        { D3DFMT_A16B16G16R16, "A16B16G16R16" }, { D3DFMT_L16, "L16" }, { D3DFMT_R16F, "R16F" }, { D3DFMT_G16R16F, "G16R16F" },
        { D3DFMT_G32R32F, "G32R32F" }, { D3DFMT_A32B32G32R32F, "A32B32G32R32F" }, { D3DFMT_Q8W8V8U8, "Q8W8V8U8" }, { D3DFMT_V16U16, "V16U16" },
        { D3DFMT_A4L4, "A4L4" }, { D3DFMT_P8, "P8" }, { D3DFMT_DXT5, "DXT5" }, { D3DFMT_D24S8, "D24S8" },
    };
    for (unsigned i = 0; i < sizeof fm / sizeof fm[0]; i++) {
        HRESULT a = IDirect3D9_CheckDeviceFormat(d3d, 0, D3DDEVTYPE_HAL, D3DFMT_X8R8G8B8, D3DUSAGE_AUTOGENMIPMAP, D3DRTYPE_TEXTURE, fm[i].f);
        HRESULT b = IDirect3D9_CheckDeviceFormat(d3d, 0, D3DDEVTYPE_HAL, D3DFMT_X8R8G8B8, D3DUSAGE_AUTOGENMIPMAP | D3DUSAGE_DYNAMIC, D3DRTYPE_TEXTURE, fm[i].f);
        HRESULT c = IDirect3D9_CheckDeviceFormat(d3d, 0, D3DDEVTYPE_HAL, D3DFMT_X8R8G8B8, D3DUSAGE_AUTOGENMIPMAP, D3DRTYPE_CUBETEXTURE, fm[i].f);
        printf("  %s %#lx %#lx cube %#lx\n", fm[i].n, a, b, c);
    }

    puts("CreateTexture D3DUSAGE_AUTOGENMIPMAP (levels, pool): hr, GetLevelCount, filter Get / Set(NONE) / Set(POINT) / Get");
    static const struct { UINT levels; D3DPOOL pool; D3DFORMAT f; } ag[] = {
        { 0, D3DPOOL_DEFAULT, D3DFMT_A8R8G8B8 }, { 1, D3DPOOL_DEFAULT, D3DFMT_A8R8G8B8 }, { 2, D3DPOOL_DEFAULT, D3DFMT_A8R8G8B8 },
        { 3, D3DPOOL_DEFAULT, D3DFMT_A8R8G8B8 }, { 0, D3DPOOL_MANAGED, D3DFMT_A8R8G8B8 }, { 0, D3DPOOL_SYSTEMMEM, D3DFMT_A8R8G8B8 },
        { 0, D3DPOOL_SCRATCH, D3DFMT_A8R8G8B8 }, { 0, D3DPOOL_MANAGED, D3DFMT_DXT1 }, { 0, D3DPOOL_DEFAULT, D3DFMT_D16 },
    };
    for (unsigned i = 0; i < sizeof ag / sizeof ag[0]; i++) {
        IDirect3DTexture9 *t = NULL;
        hr = IDirect3DDevice9_CreateTexture(dev, 64, 64, ag[i].levels, D3DUSAGE_AUTOGENMIPMAP, ag[i].f, ag[i].pool, &t, NULL);
        printf("  levels %u pool %s format %lu: %#lx", ag[i].levels, pools[ag[i].pool], (DWORD)ag[i].f, hr);
        if (SUCCEEDED(hr)) {
            DWORD f0 = IDirect3DTexture9_GetAutoGenFilterType(t);
            HRESULT n = IDirect3DTexture9_SetAutoGenFilterType(t, D3DTEXF_NONE), p = IDirect3DTexture9_SetAutoGenFilterType(t, D3DTEXF_POINT);
            printf(" levels %lu filter %lu %#lx %#lx %lu", IDirect3DTexture9_GetLevelCount(t), f0, n, p, (DWORD)IDirect3DTexture9_GetAutoGenFilterType(t));
            IDirect3DTexture9_Release(t);
        }
        printf("\n");
    }
    {
        IDirect3DTexture9 *t = NULL;
        IDirect3DDevice9_CreateTexture(dev, 16, 16, 0, 0, D3DFMT_A8R8G8B8, D3DPOOL_MANAGED, &t, NULL);
        DWORD f0 = IDirect3DTexture9_GetAutoGenFilterType(t);
        HRESULT p = IDirect3DTexture9_SetAutoGenFilterType(t, D3DTEXF_POINT);
        printf("  no AUTOGENMIPMAP: filter %lu Set(POINT) %#lx then %lu\n", f0, p, (DWORD)IDirect3DTexture9_GetAutoGenFilterType(t));
        IDirect3DTexture9_Release(t);
        IDirect3DVolumeTexture9 *v = NULL;
        printf("  CreateVolumeTexture AUTOGENMIPMAP %#lx\n", IDirect3DDevice9_CreateVolumeTexture(dev, 8, 8, 8, 0, D3DUSAGE_AUTOGENMIPMAP, D3DFMT_A8R8G8B8, D3DPOOL_DEFAULT, &v, NULL));
        if (v) IDirect3DVolumeTexture9_Release(v);
        IDirect3DCubeTexture9 *c = NULL;
        hr = IDirect3DDevice9_CreateCubeTexture(dev, 8, 0, D3DUSAGE_AUTOGENMIPMAP, D3DFMT_A8R8G8B8, D3DPOOL_DEFAULT, &c, NULL);
        printf("  CreateCubeTexture AUTOGENMIPMAP %#lx levels %lu\n", hr, c ? IDirect3DCubeTexture9_GetLevelCount(c) : 0);
        if (c) IDirect3DCubeTexture9_Release(c);
    }

    puts("the levels AUTOGENMIPMAP makes (4x4 managed, quadrants red green blue white), drawn from MAXMIPLEVEL 0, 1, 2");
    for (int filter = D3DTEXF_POINT; filter <= D3DTEXF_LINEAR; filter++) {
        IDirect3DTexture9 *t = NULL;
        hr = IDirect3DDevice9_CreateTexture(dev, 4, 4, 0, D3DUSAGE_AUTOGENMIPMAP, D3DFMT_A8R8G8B8, D3DPOOL_MANAGED, &t, NULL);
        if (FAILED(hr)) { printf("  CreateTexture %#lx\n", hr); continue; }
        IDirect3DTexture9_SetAutoGenFilterType(t, (D3DTEXTUREFILTERTYPE)filter);
        fill(t, NULL, 0, 0);
        char what[64];
        for (DWORD mip = 0; mip < 3; mip++) {
            quad((IDirect3DBaseTexture9 *)t, mip);
            sprintf(what, "filter %d MAXMIPLEVEL %lu", filter, mip);
            out(what);
        }
        /* a change of the top level: the levels again */
        RECT r = { 0, 0, 4, 4 };
        fill(t, &r, 0, 0xff808000);
        quad((IDirect3DBaseTexture9 *)t, 2);
        sprintf(what, "filter %d after a relock, MAXMIPLEVEL 2", filter);
        out(what);
        IDirect3DTexture9_GenerateMipSubLevels(t);
        quad((IDirect3DBaseTexture9 *)t, 1);
        sprintf(what, "filter %d after GenerateMipSubLevels, MAXMIPLEVEL 1", filter);
        out(what);
        IDirect3DTexture9_Release(t);
    }

    puts("dirty regions: system memory 4x4 -> default 4x4 by UpdateTexture, drawn");
    {
        IDirect3DTexture9 *s = NULL, *t = NULL;
        IDirect3DDevice9_CreateTexture(dev, 4, 4, 1, 0, D3DFMT_A8R8G8B8, D3DPOOL_SYSTEMMEM, &s, NULL);
        IDirect3DDevice9_CreateTexture(dev, 4, 4, 1, 0, D3DFMT_A8R8G8B8, D3DPOOL_DEFAULT, &t, NULL);
        fill(s, NULL, 0, 0);
        printf("  UpdateTexture %#lx\n", IDirect3DDevice9_UpdateTexture(dev, (IDirect3DBaseTexture9 *)s, (IDirect3DBaseTexture9 *)t));
        quad((IDirect3DBaseTexture9 *)t, 0);
        out("first update (all dirty)");
        RECT all = { 0, 0, 4, 4 }, tl = { 0, 0, 2, 2 }, br = { 2, 2, 4, 4 };
        fill(s, &all, D3DLOCK_NO_DIRTY_UPDATE, 0xff00ffff);
        IDirect3DDevice9_UpdateTexture(dev, (IDirect3DBaseTexture9 *)s, (IDirect3DBaseTexture9 *)t);
        quad((IDirect3DBaseTexture9 *)t, 0);
        out("after a NO_DIRTY_UPDATE lock");
        printf("  AddDirtyRect %#lx\n", IDirect3DTexture9_AddDirtyRect(s, &tl));
        IDirect3DDevice9_UpdateTexture(dev, (IDirect3DBaseTexture9 *)s, (IDirect3DBaseTexture9 *)t);
        quad((IDirect3DBaseTexture9 *)t, 0);
        out("after AddDirtyRect(top left)");
        fill(s, &br, D3DLOCK_NO_DIRTY_UPDATE, 0xffff00ff);
        fill(s, &tl, 0, 0xffffff00);
        IDirect3DDevice9_UpdateTexture(dev, (IDirect3DBaseTexture9 *)s, (IDirect3DBaseTexture9 *)t);
        quad((IDirect3DBaseTexture9 *)t, 0);
        out("after a lock of the top left");
        IDirect3DTexture9_AddDirtyRect(s, NULL);
        IDirect3DDevice9_UpdateTexture(dev, (IDirect3DBaseTexture9 *)s, (IDirect3DBaseTexture9 *)t);
        quad((IDirect3DBaseTexture9 *)t, 0);
        out("after AddDirtyRect(NULL)");
        IDirect3DTexture9_Release(s);
        IDirect3DTexture9_Release(t);
    }
    return 0;
}
