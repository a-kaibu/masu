/* Direct3D 11 NV12 textures drawn from (#227: Unity's VideoPlayer draws the reader's NV12 frames
 * through R8 and R8G8 views of their planes): an 8x4 NV12 texture of fixed bytes, its luma and
 * chroma read by a pixel shader (Load and point Sample) into an R8G8B8A8 target; then its luma
 * cleared through an R8 render target view and a chroma box updated, and drawn again; bit for
 * bit. WARP on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * run: browser (sake draws through WebGPU: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11.h>
#include <d3dcompiler.h>

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "Texture2D<float> ty : register(t0);\n"
    "Texture2D<float2> tuv : register(t1);\n"
    "SamplerState s : register(s0);\n"
    "float4 vs(uint id : SV_VertexID) : SV_Position {\n"
    "    float2 p = float2((id << 1) & 2, id & 2);\n"
    "    return float4(p * float2(2, -2) + float2(-1, 1), 0, 1);\n"
    "}\n"
    "float4 load(float4 p : SV_Position) : SV_Target {\n"
    "    int2 q = int2(p.xy);\n"
    "    return float4(ty.Load(int3(q, 0)), tuv.Load(int3(q / 2, 0)), 1);\n"
    "}\n"
    "float4 smp(float4 p : SV_Position) : SV_Target {\n"
    "    float2 uv = p.xy / float2(8, 4);\n"
    "    return float4(ty.Sample(s, uv), tuv.Sample(s, uv), 1);\n"
    "}\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "probe", NULL, NULL, entry, target, 0, 0, &b, &err);
    if (err) ID3D10Blob_Release(err);
    if (FAILED(hr)) {
        printf("D3DCompile %s failed %#lx\n", entry, hr);
        return NULL;
    }
    return b;
}

static ID3D11PixelShader *ps_of(const char *entry) {
    ID3DBlob *b = compile(entry, "ps_4_0");
    ID3D11PixelShader *p = NULL;
    if (b) ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(b), ID3D10Blob_GetBufferSize(b), NULL, &p);
    return p;
}

static ID3D11Texture2D *rt, *st;
static ID3D11RenderTargetView *rtv;

static void draw(const char *what, ID3D11PixelShader *ps, ID3D11ShaderResourceView **v) {
    static const float zero[4] = { 0 };
    ID3D11DeviceContext_ClearRenderTargetView(ctx, rtv, zero);
    ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &rtv, NULL);
    ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 2, v);
    ID3D11DeviceContext_Draw(ctx, 3, 0);
    ID3D11ShaderResourceView *none[2] = { NULL, NULL };
    ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 2, none);
    ID3D11RenderTargetView *nrt = NULL;
    ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &nrt, NULL);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)st, (ID3D11Resource *)rt);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)st, 0, D3D11_MAP_READ, 0, &m))) {
        printf("%s: Map failed\n", what);
        return;
    }
    for (int y = 0; y < 4; y++) {
        const DWORD *p = (const DWORD *)((const char *)m.pData + y * m.RowPitch);
        printf("%s row %d:", what, y);
        for (int x = 0; x < 8; x++) printf(" %08lx", p[x]);
        printf("\n");
    }
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)st, 0);
}

int main(void) {
    D3D_FEATURE_LEVEL want = D3D_FEATURE_LEVEL_11_0, got;
    HRESULT hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, &want, 1, D3D11_SDK_VERSION, &dev, &got, &ctx);
    if (FAILED(hr)) hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, &want, 1, D3D11_SDK_VERSION, &dev, &got, &ctx);
    if (FAILED(hr)) { printf("skip D3D11CreateDevice %#lx\n", hr); return 0; }
    ID3DBlob *vb = compile("vs", "vs_4_0");
    ID3D11VertexShader *vs = NULL;
    if (!vb || FAILED(ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(vb), ID3D10Blob_GetBufferSize(vb), NULL, &vs))) return 1;
    ID3D11PixelShader *pl = ps_of("load"), *pss = ps_of("smp");
    if (!pl || !pss) return 1;
    D3D11_TEXTURE2D_DESC d = { 8, 4, 1, 1, DXGI_FORMAT_R8G8B8A8_UNORM, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
    if (FAILED(ID3D11Device_CreateTexture2D(dev, &d, NULL, &rt))) return 1;
    if (FAILED(ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)rt, NULL, &rtv))) return 1;
    d.Usage = D3D11_USAGE_STAGING;
    d.BindFlags = 0;
    d.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    if (FAILED(ID3D11Device_CreateTexture2D(dev, &d, NULL, &st))) return 1;
    D3D11_VIEWPORT vp = { 0, 0, 8, 4, 0, 1 };
    ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    ID3D11DeviceContext_VSSetShader(ctx, vs, NULL, 0);
    D3D11_SAMPLER_DESC sd = { D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE_ADDRESS_CLAMP, 0, 1, D3D11_COMPARISON_NEVER, { 0 }, 0, 0 };
    ID3D11SamplerState *smp = NULL;
    ID3D11Device_CreateSamplerState(dev, &sd, &smp);
    ID3D11DeviceContext_PSSetSamplers(ctx, 0, 1, &smp);

    /* the NV12 texture: luma 8x4 (x * 16 + y * 64), chroma 4x2 pairs (u, v) */
    BYTE px[8 * 6];
    for (int y = 0; y < 4; y++)
        for (int x = 0; x < 8; x++) px[y * 8 + x] = x * 16 + y * 64;
    for (int i = 0; i < 16; i++) px[32 + i] = i & 1 ? 0xf0 - i * 8 : 0x10 + i * 8;
    D3D11_TEXTURE2D_DESC n = { 8, 4, 1, 1, DXGI_FORMAT_NV12, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET, 0, 0 };
    D3D11_SUBRESOURCE_DATA init = { px, 8, 0 };
    ID3D11Texture2D *t = NULL;
    hr = ID3D11Device_CreateTexture2D(dev, &n, &init, &t);
    printf("NV12 texture: %#lx\n", hr);
    if (!t) return 0;
    ID3D11ShaderResourceView *v[2] = { NULL, NULL };
    static const DXGI_FORMAT vf[2] = { DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8_UNORM };
    for (int i = 0; i < 2; i++) {
        D3D11_SHADER_RESOURCE_VIEW_DESC sv = { vf[i], D3D11_SRV_DIMENSION_TEXTURE2D };
        sv.Texture2D.MipLevels = 1;
        printf("view as %d: %#lx\n", vf[i], ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)t, &sv, &v[i]));
    }
    if (!v[0] || !v[1]) return 0;
    draw("load", pl, v);
    draw("sample", pss, v);
    /* luma cleared through a render target view, a chroma box (2x2 texels at (4, 0): one pair)
       updated, drawn again */
    D3D11_RENDER_TARGET_VIEW_DESC rv = { DXGI_FORMAT_R8_UNORM, D3D11_RTV_DIMENSION_TEXTURE2D };
    ID3D11RenderTargetView *lr = NULL;
    printf("luma target: %#lx\n", ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)t, &rv, &lr));
    if (lr) {
        float c[4] = { 0.25f, 0, 0, 0 };
        ID3D11DeviceContext_ClearRenderTargetView(ctx, lr, c);
        ID3D11RenderTargetView_Release(lr);
    }
    BYTE box[2 * 3] = { 1, 2, 3, 4, 0x55, 0xaa };
    D3D11_BOX b = { 4, 0, 0, 6, 2, 1 };
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)t, 0, &b, box, 2, 0);
    draw("after", pl, v);
    /* a frame-sized B8G8R8A8 texture made with its bytes (the reader's ARGB32 frames: 1920x1080,
       8 MB of initial data), its corner texels drawn */
    {
        static DWORD big[1920 * 1080];
        for (int y = 0; y < 1080; y++)
            for (int x = 0; x < 1920; x++) big[y * 1920 + x] = 0xff000000 | (x & 255) << 16 | (y & 255) << 8 | 0x40;
        D3D11_TEXTURE2D_DESC bd = { 1920, 1080, 1, 1, DXGI_FORMAT_B8G8R8A8_UNORM, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET, 0, 0 };
        D3D11_SUBRESOURCE_DATA bi = { big, 1920 * 4, 0 };
        ID3D11Texture2D *bt = NULL;
        printf("frame texture: %#lx\n", ID3D11Device_CreateTexture2D(dev, &bd, &bi, &bt));
        ID3D11ShaderResourceView *bv[2] = { NULL, NULL };
        if (bt && SUCCEEDED(ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)bt, NULL, &bv[0]))) {
            bv[1] = v[1];
            draw("frame", pl, bv);
            ID3D11ShaderResourceView_Release(bv[0]);
        }
        if (bt) ID3D11Texture2D_Release(bt);
    }
    printf("done\n");
    return 0;
}
