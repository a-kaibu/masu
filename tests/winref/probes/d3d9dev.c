/* Direct3D 9 device methods that keep state or answer for the device: GetAvailableTextureMem
 * around default pool render targets; GetGammaRamp before and after SetGammaRamp; the cursor
 * (ShowCursor, SetCursorProperties of sizes and formats); SetDialogBoxMode of each back buffer;
 * Set/GetClipStatus (and after a software-processed draw with clipping); Set/Get
 * SoftwareVertexProcessing of software, hardware and mixed devices; GetNumberOfSwapChains;
 * TestCooperativeLevel, EvictManagedResources; DrawRectPatch/DrawTriPatch/DeletePatch; an event
 * query's Issue/GetData; IDirect3D9 RegisterSoftwareDevice, CheckDeviceFormatConversion and
 * CheckDepthStencilMatch; ProcessVertices of a transformed and lit triangle.
 * libs: -ld3d9
 * arch: 32
 * run: browser (sake draws through WebGPU: tests/winref/run-browser.mjs) */
#define COBJMACROS
#include "../probe.h"
#include <d3d9.h>

#define W 64
#define H 64

static IDirect3D9 *d3d;
static HWND hwnd;

static IDirect3DDevice9 *device(DWORD behavior, DWORD flags, D3DSWAPEFFECT swap) {
    D3DPRESENT_PARAMETERS pp = { 0 };
    pp.BackBufferWidth = W; pp.BackBufferHeight = H; pp.BackBufferFormat = D3DFMT_X8R8G8B8; pp.BackBufferCount = 1;
    pp.SwapEffect = swap; pp.hDeviceWindow = hwnd; pp.Windowed = TRUE; pp.Flags = flags;
    IDirect3DDevice9 *dev = NULL;
    HRESULT hr = IDirect3D9_CreateDevice(d3d, D3DADAPTER_DEFAULT, D3DDEVTYPE_HAL, hwnd, behavior, &pp, &dev);
    if (FAILED(hr)) printf("  CreateDevice behavior %#lx flags %#lx swap %d: %#lx\n", behavior, flags, swap, hr);
    return dev;
}

static void ramp(const char *what, IDirect3DDevice9 *dev, UINT chain) {
    D3DGAMMARAMP r;
    memset(&r, 0xcd, sizeof r);
    IDirect3DDevice9_GetGammaRamp(dev, chain, &r);
    printf("  %s: red %04x %04x %04x %04x green %04x %04x blue %04x %04x\n", what, r.red[0], r.red[1], r.red[128], r.red[255], r.green[1], r.green[255], r.blue[1], r.blue[255]);
}

struct v3 { float x, y, z; DWORD c; };
struct v4 { float x, y, z, rhw; DWORD c; };

int main(void) {
    WNDCLASSA wc = { 0, DefWindowProcA, 0, 0, GetModuleHandleA(NULL), 0, 0, 0, 0, "probe" };
    RegisterClassA(&wc);
    hwnd = CreateWindowA("probe", "d3d9dev", WS_POPUP | WS_VISIBLE, 0, 0, W, H, 0, 0, wc.hInstance, 0);
    d3d = Direct3DCreate9(D3D_SDK_VERSION);
    if (!d3d) { puts("skip no Direct3D 9"); return 0; }
    IDirect3DDevice9 *dev = device(D3DCREATE_SOFTWARE_VERTEXPROCESSING, 0, D3DSWAPEFFECT_DISCARD);
    if (!dev) { puts("skip no device"); return 0; }

    puts("IDirect3D9");
    printf("  RegisterSoftwareDevice(NULL) %#lx\n", IDirect3D9_RegisterSoftwareDevice(d3d, NULL));
    static const struct { D3DFORMAT f; const char *n; } fm[] = {
        { D3DFMT_X8R8G8B8, "X8R8G8B8" }, { D3DFMT_A8R8G8B8, "A8R8G8B8" }, { D3DFMT_R5G6B5, "R5G6B5" }, { D3DFMT_X1R5G5B5, "X1R5G5B5" },
        { D3DFMT_A2R10G10B10, "A2R10G10B10" }, { D3DFMT_A16B16G16R16F, "A16B16G16R16F" }, { D3DFMT_YUY2, "YUY2" }, { D3DFMT_UYVY, "UYVY" },
        { D3DFMT_DXT1, "DXT1" }, { D3DFMT_L8, "L8" }, { D3DFMT_P8, "P8" }, { (D3DFORMAT)MAKEFOURCC('N', 'V', '1', '2'), "NV12" },
        { D3DFMT_A8, "A8" }, { D3DFMT_A8L8, "A8L8" }, { D3DFMT_A1R5G5B5, "A1R5G5B5" }, { D3DFMT_A8B8G8R8, "A8B8G8R8" }, { D3DFMT_G16R16, "G16R16" },
        { D3DFMT_R32F, "R32F" }, { D3DFMT_A32B32G32R32F, "A32B32G32R32F" }, { D3DFMT_V8U8, "V8U8" }, { D3DFMT_X8B8G8R8, "X8B8G8R8" },
        { D3DFMT_A4R4G4B4, "A4R4G4B4" }, { D3DFMT_X4R4G4B4, "X4R4G4B4" }, { D3DFMT_A2B10G10R10, "A2B10G10R10" }, { D3DFMT_A16B16G16R16, "A16B16G16R16" },
        { D3DFMT_R16F, "R16F" }, { D3DFMT_G16R16F, "G16R16F" }, { D3DFMT_G32R32F, "G32R32F" },
    };
    const int nf = sizeof fm / sizeof fm[0];
    for (int s = 0; s < nf; s++) {
        printf("  CheckDeviceFormatConversion %s ->", fm[s].n);
        for (int t = 0; t < 6; t++) printf(" %s %#lx", fm[t].n, IDirect3D9_CheckDeviceFormatConversion(d3d, 0, D3DDEVTYPE_HAL, fm[s].f, fm[t].f));
        printf("\n");
    }
    static const struct { D3DFORMAT f; const char *n; } ds[] = {
        { D3DFMT_D16, "D16" }, { D3DFMT_D24S8, "D24S8" }, { D3DFMT_D24X8, "D24X8" }, { D3DFMT_D32, "D32" }, { D3DFMT_D15S1, "D15S1" },
        { D3DFMT_D16_LOCKABLE, "D16_LOCKABLE" }, { D3DFMT_D32F_LOCKABLE, "D32F_LOCKABLE" }, { D3DFMT_X8R8G8B8, "X8R8G8B8" },
        { D3DFMT_D24X4S4, "D24X4S4" }, { D3DFMT_D24FS8, "D24FS8" },
    };
    for (int t = 0; t < 4; t++) {
        printf("  CheckDepthStencilMatch target %s:", fm[t].n);
        for (unsigned k = 0; k < sizeof ds / sizeof ds[0]; k++) printf(" %s %#lx", ds[k].n, IDirect3D9_CheckDepthStencilMatch(d3d, 0, D3DDEVTYPE_HAL, D3DFMT_X8R8G8B8, fm[t].f, ds[k].f));
        printf("\n");
    }
    printf("  CheckDeviceFormat(DEPTHSTENCIL, SURFACE):");
    for (unsigned k = 0; k < sizeof ds / sizeof ds[0]; k++) printf(" %s %#lx", ds[k].n, IDirect3D9_CheckDeviceFormat(d3d, 0, D3DDEVTYPE_HAL, D3DFMT_X8R8G8B8, D3DUSAGE_DEPTHSTENCIL, D3DRTYPE_SURFACE, ds[k].f));
    printf("\n");

    puts("GetAvailableTextureMem (MB) at first, after four 1024x1024 X8R8G8B8 render target textures, after their release");
    {
        UINT a = IDirect3DDevice9_GetAvailableTextureMem(dev);
        IDirect3DTexture9 *t[4] = { 0 };
        /* (each step's drop: the four went down by 15 MB on Windows, not 16) */
        printf("  each texture: hr, MB below the first answer:");
        for (int i = 0; i < 4; i++) {
            HRESULT hr = IDirect3DDevice9_CreateTexture(dev, 1024, 1024, 1, D3DUSAGE_RENDERTARGET, D3DFMT_X8R8G8B8, D3DPOOL_DEFAULT, &t[i], NULL);
            printf(" %#lx %u", hr, (a - IDirect3DDevice9_GetAvailableTextureMem(dev)) >> 20);
        }
        printf("\n");
        UINT b = IDirect3DDevice9_GetAvailableTextureMem(dev);
        for (int i = 0; i < 4; i++) if (t[i]) IDirect3DTexture9_Release(t[i]);
        UINT c = IDirect3DDevice9_GetAvailableTextureMem(dev);
        printf("  whole MB %d %d %d, went down by %u MB, came back %d\n", a % (1024 * 1024) == 0, b % (1024 * 1024) == 0, c % (1024 * 1024) == 0, (a - b) >> 20, c == a);
        /* (a size that is not a whole MB, and one smaller than one: how the drop rounds) */
        {
            IDirect3DTexture9 *u = NULL;
            HRESULT hr = IDirect3DDevice9_CreateTexture(dev, 1000, 1000, 1, D3DUSAGE_RENDERTARGET, D3DFMT_X8R8G8B8, D3DPOOL_DEFAULT, &u, NULL);
            printf("  1000x1000: %#lx, down by %u MB", hr, (c - IDirect3DDevice9_GetAvailableTextureMem(dev)) >> 20);
            if (u) IDirect3DTexture9_Release(u);
            u = NULL;
            hr = IDirect3DDevice9_CreateTexture(dev, 256, 256, 1, D3DUSAGE_RENDERTARGET, D3DFMT_X8R8G8B8, D3DPOOL_DEFAULT, &u, NULL);
            printf("; 256x256: %#lx, down by %u MB", hr, (c - IDirect3DDevice9_GetAvailableTextureMem(dev)) >> 20);
            if (u) IDirect3DTexture9_Release(u);
            printf(", then back %d\n", IDirect3DDevice9_GetAvailableTextureMem(dev) == c);
        }
        printf("  EvictManagedResources %#lx, then %d\n", IDirect3DDevice9_EvictManagedResources(dev), IDirect3DDevice9_GetAvailableTextureMem(dev) == c);
    }

    puts("gamma ramp (windowed)");
    ramp("before SetGammaRamp", dev, 0);
    {
        D3DGAMMARAMP r;
        for (int i = 0; i < 256; i++) { r.red[i] = (WORD)(65535 - i * 257); r.green[i] = (WORD)(i * 128); r.blue[i] = (WORD)(i * 257 / 2); }
        IDirect3DDevice9_SetGammaRamp(dev, 0, D3DSGR_NO_CALIBRATION, &r);
        ramp("after SetGammaRamp", dev, 0);
        IDirect3DDevice9_SetGammaRamp(dev, 0, D3DSGR_CALIBRATE, &r);
        ramp("after SetGammaRamp(CALIBRATE)", dev, 0);
        ramp("swap chain 1", dev, 1);
    }

    puts("cursor");
    printf("  ShowCursor(TRUE) %d %d\n", IDirect3DDevice9_ShowCursor(dev, TRUE), IDirect3DDevice9_ShowCursor(dev, TRUE));
    printf("  SetCursorProperties(NULL) %#lx\n", IDirect3DDevice9_SetCursorProperties(dev, 0, 0, NULL));
    static const struct { UINT w, h; D3DFORMAT f; D3DPOOL p; } cs[] = {
        { 32, 32, D3DFMT_A8R8G8B8, D3DPOOL_SCRATCH }, { 32, 32, D3DFMT_A8R8G8B8, D3DPOOL_SYSTEMMEM }, { 32, 32, D3DFMT_X8R8G8B8, D3DPOOL_DEFAULT },
        { 32, 32, D3DFMT_R5G6B5, D3DPOOL_DEFAULT }, { 31, 32, D3DFMT_A8R8G8B8, D3DPOOL_DEFAULT }, { 64, 16, D3DFMT_A8R8G8B8, D3DPOOL_DEFAULT },
        { 1, 1, D3DFMT_A8R8G8B8, D3DPOOL_DEFAULT }, { 32, 32, D3DFMT_A8R8G8B8, D3DPOOL_DEFAULT },
    };
    for (unsigned i = 0; i < sizeof cs / sizeof cs[0]; i++) {
        IDirect3DSurface9 *s = NULL;
        HRESULT hc = IDirect3DDevice9_CreateOffscreenPlainSurface(dev, cs[i].w, cs[i].h, cs[i].f, cs[i].p, &s, NULL);
        HRESULT hr = s ? IDirect3DDevice9_SetCursorProperties(dev, 1, 2, s) : E_FAIL;
        printf("  %ux%u format %lu pool %d: create %#lx SetCursorProperties %#lx\n", cs[i].w, cs[i].h, (DWORD)cs[i].f, cs[i].p, hc, hr);
        if (s) IDirect3DSurface9_Release(s);
    }
    {
        int a = IDirect3DDevice9_ShowCursor(dev, TRUE), b = IDirect3DDevice9_ShowCursor(dev, FALSE), c = IDirect3DDevice9_ShowCursor(dev, FALSE);
        printf("  ShowCursor(TRUE, FALSE, FALSE) %d %d %d\n", a, b, c);
    }

    puts("SetDialogBoxMode(TRUE) / (FALSE)");
    printf("  no LOCKABLE_BACKBUFFER: %#lx %#lx\n", IDirect3DDevice9_SetDialogBoxMode(dev, TRUE), IDirect3DDevice9_SetDialogBoxMode(dev, FALSE));
    {
        static const struct { DWORD flags; D3DSWAPEFFECT swap; const char *n; } dm[] = {
            { D3DPRESENTFLAG_LOCKABLE_BACKBUFFER, D3DSWAPEFFECT_DISCARD, "LOCKABLE DISCARD" }, { D3DPRESENTFLAG_LOCKABLE_BACKBUFFER, D3DSWAPEFFECT_COPY, "LOCKABLE COPY" },
            { D3DPRESENTFLAG_LOCKABLE_BACKBUFFER, D3DSWAPEFFECT_FLIP, "LOCKABLE FLIP" }, { 0, D3DSWAPEFFECT_COPY, "COPY" },
        };
        for (unsigned i = 0; i < sizeof dm / sizeof dm[0]; i++) {
            IDirect3DDevice9 *d2 = device(D3DCREATE_SOFTWARE_VERTEXPROCESSING, dm[i].flags, dm[i].swap);
            if (!d2) continue;
            printf("  %s: %#lx %#lx\n", dm[i].n, IDirect3DDevice9_SetDialogBoxMode(d2, TRUE), IDirect3DDevice9_SetDialogBoxMode(d2, FALSE));
            IDirect3DDevice9_Release(d2);
        }
    }

    puts("clip status");
    {
        D3DCLIPSTATUS9 c = { 0xcdcdcdcd, 0xcdcdcdcd };
        HRESULT hr = IDirect3DDevice9_GetClipStatus(dev, &c);
        printf("  at first %#lx %#lx %#lx\n", hr, c.ClipUnion, c.ClipIntersection);
        D3DCLIPSTATUS9 s = { 0x12, 0x34 };
        printf("  SetClipStatus %#lx", IDirect3DDevice9_SetClipStatus(dev, &s));
        IDirect3DDevice9_GetClipStatus(dev, &c);
        printf(" then %#lx %#lx\n", c.ClipUnion, c.ClipIntersection);
        /* (GetClipStatus(NULL) / SetClipStatus(NULL) kill the process on Windows, exit 139: not asked) */
        /* a lit triangle partly left of the view, through the fixed function, software processed */
        D3DMATRIX id = { { { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1 } } };
        IDirect3DDevice9_SetTransform(dev, D3DTS_WORLD, &id);
        IDirect3DDevice9_SetTransform(dev, D3DTS_VIEW, &id);
        IDirect3DDevice9_SetTransform(dev, D3DTS_PROJECTION, &id);
        IDirect3DDevice9_SetRenderState(dev, D3DRS_LIGHTING, FALSE);
        IDirect3DDevice9_SetRenderState(dev, D3DRS_CLIPPING, TRUE);
        struct v3 tri[3] = { { -2.0f, 0.0f, 0.5f, 0xffffffff }, { 0.5f, 0.5f, 0.5f, 0xffffffff }, { 0.5f, -0.5f, 0.5f, 0xffffffff } };
        IDirect3DDevice9_SetFVF(dev, D3DFVF_XYZ | D3DFVF_DIFFUSE);
        s.ClipUnion = 0; s.ClipIntersection = 0xffffffff;
        IDirect3DDevice9_SetClipStatus(dev, &s);
        IDirect3DDevice9_BeginScene(dev);
        IDirect3DDevice9_DrawPrimitiveUP(dev, D3DPT_TRIANGLELIST, 1, tri, sizeof tri[0]);
        IDirect3DDevice9_EndScene(dev);
        IDirect3DDevice9_GetClipStatus(dev, &c);
        printf("  after a draw partly left of the view: %#lx %#lx\n", c.ClipUnion, c.ClipIntersection);
    }

    puts("software vertex processing: Get, Set(FALSE), Get, Set(TRUE), Get");
    {
        static const struct { DWORD b; const char *n; } vp[] = {
            { D3DCREATE_SOFTWARE_VERTEXPROCESSING, "software" }, { D3DCREATE_HARDWARE_VERTEXPROCESSING, "hardware" }, { D3DCREATE_MIXED_VERTEXPROCESSING, "mixed" },
        };
        for (int i = 0; i < 3; i++) {
            IDirect3DDevice9 *d2 = device(vp[i].b, 0, D3DSWAPEFFECT_DISCARD);
            if (!d2) continue;
            BOOL a = IDirect3DDevice9_GetSoftwareVertexProcessing(d2);
            HRESULT h0 = IDirect3DDevice9_SetSoftwareVertexProcessing(d2, FALSE);
            BOOL b = IDirect3DDevice9_GetSoftwareVertexProcessing(d2);
            HRESULT h1 = IDirect3DDevice9_SetSoftwareVertexProcessing(d2, TRUE);
            BOOL c = IDirect3DDevice9_GetSoftwareVertexProcessing(d2);
            printf("  %s: %d %#lx %d %#lx %d\n", vp[i].n, a, h0, b, h1, c);
            IDirect3DDevice9_Release(d2);
        }
    }

    puts("swap chains, cooperative level");
    {
        printf("  GetNumberOfSwapChains %u", IDirect3DDevice9_GetNumberOfSwapChains(dev));
        D3DPRESENT_PARAMETERS pp = { 0 };
        pp.BackBufferWidth = 16; pp.BackBufferHeight = 16; pp.BackBufferFormat = D3DFMT_X8R8G8B8; pp.SwapEffect = D3DSWAPEFFECT_DISCARD; pp.hDeviceWindow = hwnd; pp.Windowed = TRUE;
        IDirect3DSwapChain9 *sc = NULL;
        HRESULT hr = IDirect3DDevice9_CreateAdditionalSwapChain(dev, &pp, &sc);
        printf(", CreateAdditionalSwapChain %#lx, then %u\n", hr, IDirect3DDevice9_GetNumberOfSwapChains(dev));
        if (sc) IDirect3DSwapChain9_Release(sc);
        printf("  TestCooperativeLevel %#lx\n", IDirect3DDevice9_TestCooperativeLevel(dev));
    }

    puts("patches");
    {
        D3DCAPS9 caps;
        IDirect3DDevice9_GetDeviceCaps(dev, &caps);
        printf("  DevCaps RTPATCHES %d NPATCHES %d QUINTICRTPATCHES %d MaxNpatchTessellationLevel %g\n", !!(caps.DevCaps & D3DDEVCAPS_RTPATCHES), !!(caps.DevCaps & D3DDEVCAPS_NPATCHES), !!(caps.DevCaps & D3DDEVCAPS_QUINTICRTPATCHES), caps.MaxNpatchTessellationLevel);
        float segs[4] = { 2, 2, 2, 2 };
        D3DRECTPATCH_INFO ri = { 0, 0, 4, 4, 4, D3DBASIS_BEZIER, D3DDEGREE_CUBIC };
        D3DTRIPATCH_INFO ti = { 0, 10, D3DBASIS_BEZIER, D3DDEGREE_CUBIC };
        printf("  DrawRectPatch(0, segs, info) %#lx DrawRectPatch(5, segs, info) %#lx DrawRectPatch(5, segs, NULL) %#lx\n",
            IDirect3DDevice9_DrawRectPatch(dev, 0, segs, &ri), IDirect3DDevice9_DrawRectPatch(dev, 5, segs, &ri), IDirect3DDevice9_DrawRectPatch(dev, 5, segs, NULL));
        printf("  DrawTriPatch(0, segs, info) %#lx DrawTriPatch(6, segs, info) %#lx\n", IDirect3DDevice9_DrawTriPatch(dev, 0, segs, &ti), IDirect3DDevice9_DrawTriPatch(dev, 6, segs, &ti));
        printf("  DeletePatch(5) %#lx DeletePatch(7) %#lx DeletePatch(0) %#lx\n", IDirect3DDevice9_DeletePatch(dev, 5), IDirect3DDevice9_DeletePatch(dev, 7), IDirect3DDevice9_DeletePatch(dev, 0));
    }

    puts("event query: GetData before Issue, Issue(END), GetData(FLUSH) until done, Issue(BEGIN)");
    {
        IDirect3DQuery9 *q = NULL;
        HRESULT hr = IDirect3DDevice9_CreateQuery(dev, D3DQUERYTYPE_EVENT, &q);
        if (q) {
            BOOL done = 7;
            HRESULT g0 = IDirect3DQuery9_GetData(q, &done, sizeof done, 0);
            HRESULT i1 = IDirect3DQuery9_Issue(q, D3DISSUE_END);
            int n = 0;
            HRESULT g1;
            while ((g1 = IDirect3DQuery9_GetData(q, &done, sizeof done, D3DGETDATA_FLUSH)) == S_FALSE && n < 100000) n++;
            printf("  CreateQuery %#lx GetData %#lx Issue(END) %#lx GetData %#lx done %d (loops < 100000 %d) Issue(BEGIN) %#lx Issue(0) %#lx\n", hr, g0, i1, g1, done, n < 100000,
                IDirect3DQuery9_Issue(q, D3DISSUE_BEGIN), IDirect3DQuery9_Issue(q, 0));
            IDirect3DQuery9_Release(q);
        } else printf("  CreateQuery %#lx\n", hr);
    }

    puts("ProcessVertices (XYZ | DIFFUSE through identity transforms, viewport 64x64, into XYZRHW | DIFFUSE)");
    {
        IDirect3DVertexBuffer9 *src = NULL, *dst = NULL;
        IDirect3DDevice9_CreateVertexBuffer(dev, 3 * sizeof(struct v3), 0, D3DFVF_XYZ | D3DFVF_DIFFUSE, D3DPOOL_SYSTEMMEM, &src, NULL);
        IDirect3DDevice9_CreateVertexBuffer(dev, 4 * sizeof(struct v4), 0, D3DFVF_XYZRHW | D3DFVF_DIFFUSE, D3DPOOL_SYSTEMMEM, &dst, NULL);
        struct v3 *p;
        IDirect3DVertexBuffer9_Lock(src, 0, 0, (void **)&p, 0);
        struct v3 tri[3] = { { -0.5f, 0.5f, 0.25f, 0xff102030 }, { 0.5f, 0.5f, 0.5f, 0xff405060 }, { 0.75f, -1.5f, 0.75f, 0x80708090 } };
        memcpy(p, tri, sizeof tri);
        IDirect3DVertexBuffer9_Unlock(src);
        struct v4 *q;
        IDirect3DVertexBuffer9_Lock(dst, 0, 0, (void **)&q, 0);
        memset(q, 0xcd, 4 * sizeof(struct v4));
        IDirect3DVertexBuffer9_Unlock(dst);
        IDirect3DDevice9_SetStreamSource(dev, 0, src, 0, sizeof(struct v3));
        IDirect3DDevice9_SetFVF(dev, D3DFVF_XYZ | D3DFVF_DIFFUSE);
        HRESULT hr = IDirect3DDevice9_ProcessVertices(dev, 0, 1, 3, dst, NULL, 0);
        printf("  ProcessVertices %#lx\n", hr);
        IDirect3DVertexBuffer9_Lock(dst, 0, 0, (void **)&q, D3DLOCK_READONLY);
        for (int i = 0; i < 4; i++) printf("  [%d] %g %g %g %g %08lx\n", i, q[i].x, q[i].y, q[i].z, q[i].rhw, q[i].c);
        IDirect3DVertexBuffer9_Unlock(dst);
        printf("  into a buffer of another FVF (XYZ): %#lx\n", IDirect3DDevice9_ProcessVertices(dev, 0, 0, 3, src, NULL, 0));
        IDirect3DVertexBuffer9_Release(src);
        IDirect3DVertexBuffer9_Release(dst);
    }
    IDirect3DDevice9_Release(dev);
    return 0;
}
