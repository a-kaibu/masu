/* Direct3D 9 render target and depth-stencil cube textures: CheckDeviceFormat and
 * CreateCubeTexture of each usage and format; each face of an 8x8 two-level render target cube
 * cleared, drawn to (left half), color filled (a rectangle) and stretched into from another
 * face, then read back (GetRenderTargetData) and sampled (a direction toward each face); a
 * depth-stencil cube's face as the depth buffer of a draw.
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

struct cv { float x, y, z, rhw; DWORD c; };
struct dv { float x, y, z, rhw; float u, v, w; };

static void rect(float x0, float y0, float x1, float y1, float z, DWORD c) {
    struct cv v[4] = { { x0 - 0.5f, y0 - 0.5f, z, 1, c }, { x1 - 0.5f, y0 - 0.5f, z, 1, c }, { x0 - 0.5f, y1 - 0.5f, z, 1, c }, { x1 - 0.5f, y1 - 0.5f, z, 1, c } };
    IDirect3DDevice9_SetFVF(dev, D3DFVF_XYZRHW | D3DFVF_DIFFUSE);
    IDirect3DDevice9_DrawPrimitiveUP(dev, D3DPT_TRIANGLESTRIP, 2, v, sizeof v[0]);
}

/* the back buffer's pixel (x, y), without alpha */
static DWORD bpx(int x, int y) {
    D3DLOCKED_RECT lr;
    if (FAILED(IDirect3DDevice9_GetRenderTargetData(dev, back, bsys))) return 0xbad0bad0;
    if (FAILED(IDirect3DSurface9_LockRect(bsys, &lr, NULL, D3DLOCK_READONLY))) return 0xbad1bad1;
    DWORD v = ((DWORD *)((char *)lr.pBits + y * lr.Pitch))[x] & 0xffffff;
    IDirect3DSurface9_UnlockRect(bsys);
    return v;
}

/* pixels (1, 1), (6, 1), (3, 5), (6, 6) of a face's level, read back */
static void faceout(IDirect3DCubeTexture9 *c, int face, int level) {
    IDirect3DSurface9 *s = NULL, *sys = NULL;
    int e = W >> level;
    IDirect3DCubeTexture9_GetCubeMapSurface(c, (D3DCUBEMAP_FACES)face, level, &s);
    IDirect3DDevice9_CreateOffscreenPlainSurface(dev, e, e, D3DFMT_A8R8G8B8, D3DPOOL_SYSTEMMEM, &sys, NULL);
    HRESULT hr = s && sys ? IDirect3DDevice9_GetRenderTargetData(dev, s, sys) : E_FAIL;
    printf("  face %d level %d GetRenderTargetData %#lx", face, level, hr);
    D3DLOCKED_RECT lr;
    if (SUCCEEDED(hr) && SUCCEEDED(IDirect3DSurface9_LockRect(sys, &lr, NULL, D3DLOCK_READONLY))) {
        static const int at[4][2] = { { 1, 1 }, { 6, 1 }, { 3, 5 }, { 6, 6 } };
        for (int k = 0; k < 4; k++) {
            int x = at[k][0] * e / W, y = at[k][1] * e / H;
            printf(" %08lx", ((DWORD *)((char *)lr.pBits + y * lr.Pitch))[x]);
        }
        IDirect3DSurface9_UnlockRect(sys);
    }
    printf("\n");
    if (sys) IDirect3DSurface9_Release(sys);
    if (s) IDirect3DSurface9_Release(s);
}

static const struct { D3DFORMAT f; const char *n; } fm[] = {
    { D3DFMT_A8R8G8B8, "A8R8G8B8" }, { D3DFMT_X8R8G8B8, "X8R8G8B8" }, { D3DFMT_R5G6B5, "R5G6B5" }, { D3DFMT_X8B8G8R8, "X8B8G8R8" },
    { D3DFMT_A16B16G16R16F, "A16B16G16R16F" }, { D3DFMT_R32F, "R32F" }, { D3DFMT_DXT1, "DXT1" }, { D3DFMT_D16, "D16" },
    { D3DFMT_D24S8, "D24S8" }, { D3DFMT_D24X8, "D24X8" }, { D3DFMT_D32, "D32" },
};

static const DWORD faces[6] = { 0xff800000, 0xff008000, 0xff000080, 0xff808000, 0xff008080, 0xff800080 };

int main(void) {
    WNDCLASSA wc = { 0, DefWindowProcA, 0, 0, GetModuleHandleA(NULL), 0, 0, 0, 0, "probe" };
    RegisterClassA(&wc);
    HWND hwnd = CreateWindowA("probe", "d3d9cube", WS_POPUP, 0, 0, W, H, 0, 0, wc.hInstance, 0);
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
    IDirect3DDevice9_SetRenderState(dev, D3DRS_ZENABLE, D3DZB_FALSE);
    IDirect3DDevice9_SetSamplerState(dev, 0, D3DSAMP_MINFILTER, D3DTEXF_POINT);
    IDirect3DDevice9_SetSamplerState(dev, 0, D3DSAMP_MAGFILTER, D3DTEXF_POINT);
    IDirect3DDevice9_SetSamplerState(dev, 0, D3DSAMP_MIPFILTER, D3DTEXF_NONE);

    /* each format and usage: CheckDeviceFormat and CreateCubeTexture (default pool; a managed
     * render target cube) */
    for (int i = 0; i < (int)(sizeof fm / sizeof fm[0]); i++) {
        static const DWORD us[3] = { 0, D3DUSAGE_RENDERTARGET, D3DUSAGE_DEPTHSTENCIL };
        printf("%s", fm[i].n);
        for (int u = 0; u < 3; u++) {
            HRESULT c = IDirect3D9_CheckDeviceFormat(d3d, 0, D3DDEVTYPE_HAL, D3DFMT_X8R8G8B8, us[u], D3DRTYPE_CUBETEXTURE, fm[i].f);
            IDirect3DCubeTexture9 *t = (IDirect3DCubeTexture9 *)1;
            hr = IDirect3DDevice9_CreateCubeTexture(dev, 4, 1, us[u], fm[i].f, D3DPOOL_DEFAULT, &t, NULL);
            printf(" usage %#lx check %d create %#lx %s", us[u], c == D3D_OK, hr, t == NULL ? "null" : t == (IDirect3DCubeTexture9 *)1 ? "untouched" : "made");
            if (SUCCEEDED(hr) && t) {
                D3DSURFACE_DESC d;
                IDirect3DCubeTexture9_GetLevelDesc(t, 0, &d);
                printf(" desc %u/%u/%#lx/%u", d.Format, d.Type, d.Usage, d.Pool);
                IDirect3DCubeTexture9_Release(t);
            }
        }
        IDirect3DCubeTexture9 *t = NULL;
        hr = IDirect3DDevice9_CreateCubeTexture(dev, 4, 1, D3DUSAGE_RENDERTARGET, fm[i].f, D3DPOOL_MANAGED, &t, NULL);
        printf(" managed rendertarget %#lx\n", hr);
        if (t) IDirect3DCubeTexture9_Release(t);
    }

    /* an 8x8 render target cube of two levels: each face of each level cleared and drawn to */
    IDirect3DCubeTexture9 *cube = NULL;
    hr = IDirect3DDevice9_CreateCubeTexture(dev, W, 2, D3DUSAGE_RENDERTARGET, D3DFMT_A8R8G8B8, D3DPOOL_DEFAULT, &cube, NULL);
    printf("render target cube %#lx levels %lu\n", hr, cube ? IDirect3DCubeTexture9_GetLevelCount(cube) : 0);
    if (FAILED(hr)) return 0;
    IDirect3DDevice9_SetTextureStageState(dev, 0, D3DTSS_COLOROP, D3DTOP_SELECTARG1);
    IDirect3DDevice9_SetTextureStageState(dev, 0, D3DTSS_COLORARG1, D3DTA_DIFFUSE);
    IDirect3DDevice9_SetTextureStageState(dev, 0, D3DTSS_ALPHAOP, D3DTOP_SELECTARG1);
    IDirect3DDevice9_SetTextureStageState(dev, 0, D3DTSS_ALPHAARG1, D3DTA_DIFFUSE);
    for (int l = 0; l < 2; l++) {
        for (int f = 0; f < 6; f++) {
            IDirect3DSurface9 *s = NULL;
            IDirect3DCubeTexture9_GetCubeMapSurface(cube, (D3DCUBEMAP_FACES)f, l, &s);
            HRESULT st = IDirect3DDevice9_SetRenderTarget(dev, 0, s);
            D3DVIEWPORT9 vp;
            IDirect3DDevice9_GetViewport(dev, &vp);
            HRESULT cl = IDirect3DDevice9_Clear(dev, 0, NULL, D3DCLEAR_TARGET, faces[f] + 0x10 * l, 1, 0);
            IDirect3DDevice9_BeginScene(dev);
            int e = W >> l;
            rect(0, 0, e / 2.0f, (float)e, 0, 0xff00ff00 | (f * 0x20 + l * 0x10));
            IDirect3DDevice9_EndScene(dev);
            printf("face %d level %d SetRenderTarget %#lx viewport %lux%lu Clear %#lx\n", f, l, st, vp.Width, vp.Height, cl);
            IDirect3DSurface9_Release(s);
        }
    }
    IDirect3DDevice9_SetRenderTarget(dev, 0, back);
    for (int l = 0; l < 2; l++)
        for (int f = 0; f < 6; f++) faceout(cube, f, l);

    /* ColorFill a rectangle of face 2, StretchRect face 0 level 0 into the right half of face 5 */
    {
        IDirect3DSurface9 *s2 = NULL, *s0 = NULL, *s5 = NULL;
        IDirect3DCubeTexture9_GetCubeMapSurface(cube, D3DCUBEMAP_FACE_POSITIVE_Y, 0, &s2);
        IDirect3DCubeTexture9_GetCubeMapSurface(cube, D3DCUBEMAP_FACE_POSITIVE_X, 0, &s0);
        IDirect3DCubeTexture9_GetCubeMapSurface(cube, D3DCUBEMAP_FACE_NEGATIVE_Z, 0, &s5);
        RECT r = { 2, 4, 8, 8 };
        /* (Windows' device was lost after these two, GetRenderTargetData D3DERR_DEVICELOST: each is
         * followed by TestCooperativeLevel and its face's read back, to tell which one) */
        printf("ColorFill %#lx", IDirect3DDevice9_ColorFill(dev, s2, &r, 0xff123456));
        printf(" TestCooperativeLevel %#lx\n", IDirect3DDevice9_TestCooperativeLevel(dev));
        faceout(cube, 2, 0);
        RECT sr = { 0, 0, 8, 8 }, dr = { 4, 0, 8, 8 };
        printf("StretchRect %#lx", IDirect3DDevice9_StretchRect(dev, s0, &sr, s5, &dr, D3DTEXF_POINT));
        printf(" TestCooperativeLevel %#lx\n", IDirect3DDevice9_TestCooperativeLevel(dev));
        faceout(cube, 5, 0);
        IDirect3DSurface9_Release(s2);
        IDirect3DSurface9_Release(s0);
        IDirect3DSurface9_Release(s5);
    }

    /* sampled: a quad with a direction toward each face's center, then each face's left half */
    IDirect3DDevice9_SetTexture(dev, 0, (IDirect3DBaseTexture9 *)cube);
    IDirect3DDevice9_SetTextureStageState(dev, 0, D3DTSS_COLORARG1, D3DTA_TEXTURE);
    IDirect3DDevice9_SetFVF(dev, D3DFVF_XYZRHW | D3DFVF_TEX1 | D3DFVF_TEXCOORDSIZE3(0));
    static const float dir[6][3] = { { 1, 0, 0 }, { -1, 0, 0 }, { 0, 1, 0 }, { 0, -1, 0 }, { 0, 0, 1 }, { 0, 0, -1 } };
    printf("sampled");
    for (int f = 0; f < 6; f++) {
        IDirect3DDevice9_Clear(dev, 0, NULL, D3DCLEAR_TARGET, 0x123456, 1, 0);
        struct dv v[4];
        for (int k = 0; k < 4; k++) {
            v[k].x = (k & 1 ? W : 0) - 0.5f;
            v[k].y = (k & 2 ? H : 0) - 0.5f;
            v[k].z = 0;
            v[k].rhw = 1;
            v[k].u = dir[f][0];
            v[k].v = dir[f][1];
            v[k].w = dir[f][2];
        }
        IDirect3DDevice9_BeginScene(dev);
        IDirect3DDevice9_DrawPrimitiveUP(dev, D3DPT_TRIANGLESTRIP, 2, v, sizeof v[0]);
        IDirect3DDevice9_EndScene(dev);
        printf(" %06lx", bpx(4, 4));
    }
    printf("\n");
    IDirect3DDevice9_SetTexture(dev, 0, NULL);
    IDirect3DDevice9_SetTextureStageState(dev, 0, D3DTSS_COLORARG1, D3DTA_DIFFUSE);

    /* a depth-stencil cube's face as the depth buffer of an 8x8 render target */
    IDirect3DCubeTexture9 *zc = NULL;
    hr = IDirect3DDevice9_CreateCubeTexture(dev, W, 1, D3DUSAGE_DEPTHSTENCIL, D3DFMT_D24S8, D3DPOOL_DEFAULT, &zc, NULL);
    printf("depth cube %#lx\n", hr);
    if (SUCCEEDED(hr)) {
        IDirect3DSurface9 *z = NULL;
        IDirect3DCubeTexture9_GetCubeMapSurface(zc, D3DCUBEMAP_FACE_NEGATIVE_Y, 0, &z);
        printf("SetDepthStencilSurface %#lx\n", IDirect3DDevice9_SetDepthStencilSurface(dev, z));
        IDirect3DDevice9_SetRenderState(dev, D3DRS_ZENABLE, D3DZB_TRUE);
        IDirect3DDevice9_SetRenderState(dev, D3DRS_ZFUNC, D3DCMP_LESS);
        printf("Clear %#lx\n", IDirect3DDevice9_Clear(dev, 0, NULL, D3DCLEAR_TARGET | D3DCLEAR_ZBUFFER | D3DCLEAR_STENCIL, 0x000000, 0.5f, 0));
        IDirect3DDevice9_BeginScene(dev);
        rect(0, 0, 8, 4, 0.3f, 0xffff0000);
        rect(0, 4, 8, 8, 0.7f, 0xff0000ff);
        rect(4, 0, 8, 8, 0.4f, 0xff00ff00);
        IDirect3DDevice9_EndScene(dev);
        printf("depth tested %06lx %06lx %06lx %06lx\n", bpx(1, 1), bpx(1, 6), bpx(6, 1), bpx(6, 6));
        IDirect3DDevice9_SetDepthStencilSurface(dev, NULL);
        IDirect3DSurface9_Release(z);
        IDirect3DCubeTexture9_Release(zc);
    }
    IDirect3DCubeTexture9_Release(cube);
    return 0;
}
