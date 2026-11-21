/* Direct3D 11 per-resource mip clamps (ID3D11DeviceContext::SetResourceMinLOD /
 * GetResourceMinLOD): what GetResourceMinLOD gives back after each value Set (fractions, past
 * the last mip, negative, NaN, a buffer, a 3D texture), and what a shader reads of an 8x8
 * texture of 4 mips (mip k holds (k + 1) * 50 in red) under each clamp: SampleLevel at fixed
 * LODs (point and linear mip filters), Sample with a derivative LOD of 1, Load of each mip,
 * GetDimensions; through a view whose most detailed mip is 1 too; a sampler's MinLOD with it.
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11.h>
#include <d3dcompiler.h>
#include <math.h>

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "Texture2D<float4> t : register(t0);\n"
    "SamplerState s : register(s0);\n"
    "cbuffer c : register(b0) { float lod; int mip; float2 pad; };\n"
    "float4 vs(uint id : SV_VertexID) : SV_Position {\n"
    "    float2 p = float2((id << 1) & 2, id & 2);\n"
    "    return float4(p * float2(2, -2) + float2(-1, 1), 0, 1);\n"
    "}\n"
    /* (the 4x4 target covers the 8x8 texture: a derivative LOD of 1) */
    "float4 level(float4 p : SV_Position) : SV_Target { return t.SampleLevel(s, p.xy / 4, lod); }\n"
    "float4 sample(float4 p : SV_Position) : SV_Target { return t.Sample(s, p.xy / 4); }\n"
    "float4 load(float4 p : SV_Position) : SV_Target { return t.Load(int3(0, 0, mip)); }\n"
    "float4 dims(float4 p : SV_Position) : SV_Target {\n"
    "    uint w, h, n; t.GetDimensions(mip, w, h, n); return float4(w, h, n, 0);\n"
    "}\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "minlod", NULL, NULL, entry, target, 0, 0, &b, &err);
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

static ID3D11Texture2D *rt, *staging;

/* a line put together while draws run, printed whole (stdout is unbuffered: a line printed in
 * pieces between GPU waits is split on the page's console) */
static char line[512];
static void add(const char *fmt, double v) {
    size_t n = strlen(line);
    snprintf(line + n, sizeof line - n, fmt, v);
}
static void add3(const char *fmt, double a, double b, double c) {
    size_t n = strlen(line);
    snprintf(line + n, sizeof line - n, fmt, a, b, c);
}
static void end_line(void) {
    puts(line);
    line[0] = 0;
}
static ID3D11Buffer *cb;

/* pixel (0, 0) of the target after a draw with `ps` */
static void run(ID3D11PixelShader *ps, float lod, int mip, float out[4]) {
    float c[4] = { lod, 0, 0, 0 };
    memcpy(&c[1], &mip, 4);
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)cb, 0, NULL, c, 0, 0);
    ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    ID3D11DeviceContext_Draw(ctx, 3, 0);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)staging, (ID3D11Resource *)rt);
    D3D11_MAPPED_SUBRESOURCE m;
    memset(out, 0, 16);
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)staging, 0, D3D11_MAP_READ, 0, &m))) return;
    memcpy(out, m.pData, 16);
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)staging, 0);
}

static ID3D11SamplerState *sampler(D3D11_FILTER f, float min_lod) {
    D3D11_SAMPLER_DESC d = { f, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE_ADDRESS_CLAMP, 0, 1, D3D11_COMPARISON_NEVER, { 0, 0, 0, 0 }, min_lod, D3D11_FLOAT32_MAX };
    ID3D11SamplerState *s = NULL;
    ID3D11Device_CreateSamplerState(dev, &d, &s);
    return s;
}

int main(void) {
    D3D_FEATURE_LEVEL fl;
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx)) &&
        FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx))) {
        puts("skip no device");
        return 0;
    }
    /* the texture: 8x8, 4 mips, mip k all (k + 1) * 50 in red */
    D3D11_TEXTURE2D_DESC td = { 8, 8, 4, 1, DXGI_FORMAT_R8G8B8A8_UNORM, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_SHADER_RESOURCE, 0, 0 };
    static UINT px[4][64];
    D3D11_SUBRESOURCE_DATA init[4];
    for (int k = 0; k < 4; k++) {
        for (int i = 0; i < 64; i++) px[k][i] = (UINT)((k + 1) * 50) | 0xff000000u;
        init[k].pSysMem = px[k];
        init[k].SysMemPitch = (8 >> k) * 4;
        init[k].SysMemSlicePitch = 0;
    }
    ID3D11Texture2D *tex = NULL;
    if (FAILED(ID3D11Device_CreateTexture2D(dev, &td, init, &tex))) { puts("CreateTexture2D failed"); return 0; }

    puts("GetResourceMinLOD after SetResourceMinLOD (texture of 4 mips)");
    printf("  fresh: %g\n", ID3D11DeviceContext_GetResourceMinLOD(ctx, (ID3D11Resource *)tex));
    {
        const float v[] = { 0, 0.5f, 1, 2.75f, 3, 3.5f, 4, 10, -1, -0.5f, D3D11_FLOAT32_MAX, NAN, INFINITY, -INFINITY };
        for (unsigned i = 0; i < sizeof v / sizeof *v; i++) {
            ID3D11DeviceContext_SetResourceMinLOD(ctx, (ID3D11Resource *)tex, v[i]);
            float g = ID3D11DeviceContext_GetResourceMinLOD(ctx, (ID3D11Resource *)tex);
            UINT bits;
            memcpy(&bits, &g, 4);
            printf("  set %g: %g (%08x)\n", v[i], g, bits);
        }
    }
    {
        D3D11_BUFFER_DESC bd = { 64, D3D11_USAGE_DEFAULT, D3D11_BIND_VERTEX_BUFFER, 0, 0, 0 };
        ID3D11Buffer *b = NULL;
        ID3D11Device_CreateBuffer(dev, &bd, NULL, &b);
        ID3D11DeviceContext_SetResourceMinLOD(ctx, (ID3D11Resource *)b, 2);
        printf("  buffer set 2: %g\n", ID3D11DeviceContext_GetResourceMinLOD(ctx, (ID3D11Resource *)b));
        ID3D11Buffer_Release(b);
        D3D11_TEXTURE3D_DESC d3 = { 4, 4, 4, 3, DXGI_FORMAT_R8G8B8A8_UNORM, D3D11_USAGE_DEFAULT, D3D11_BIND_SHADER_RESOURCE, 0, 0 };
        ID3D11Texture3D *t3 = NULL;
        ID3D11Device_CreateTexture3D(dev, &d3, NULL, &t3);
        if (t3) {
            ID3D11DeviceContext_SetResourceMinLOD(ctx, (ID3D11Resource *)t3, 1.25f);
            printf("  3D texture set 1.25: %g\n", ID3D11DeviceContext_GetResourceMinLOD(ctx, (ID3D11Resource *)t3));
            ID3D11Texture3D_Release(t3);
        }
    }

    /* the target and the pipeline */
    D3D11_TEXTURE2D_DESC rd = { 4, 4, 1, 1, DXGI_FORMAT_R32G32B32A32_FLOAT, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
    ID3D11Device_CreateTexture2D(dev, &rd, NULL, &rt);
    rd.Usage = D3D11_USAGE_STAGING;
    rd.BindFlags = 0;
    rd.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    ID3D11Device_CreateTexture2D(dev, &rd, NULL, &staging);
    ID3D11RenderTargetView *rtv = NULL;
    ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)rt, NULL, &rtv);
    ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &rtv, NULL);
    D3D11_VIEWPORT vp = { 0, 0, 4, 4, 0, 1 };
    ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
    ID3DBlob *vb = compile("vs", "vs_4_0");
    ID3D11VertexShader *vs = NULL;
    if (vb) ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(vb), ID3D10Blob_GetBufferSize(vb), NULL, &vs);
    ID3D11DeviceContext_VSSetShader(ctx, vs, NULL, 0);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    D3D11_BUFFER_DESC cbd = { 16, D3D11_USAGE_DEFAULT, D3D11_BIND_CONSTANT_BUFFER, 0, 0, 0 };
    ID3D11Device_CreateBuffer(dev, &cbd, NULL, &cb);
    ID3D11DeviceContext_PSSetConstantBuffers(ctx, 0, 1, &cb);
    ID3D11PixelShader *ps_level = ps_of("level"), *ps_sample = ps_of("sample"), *ps_load = ps_of("load"), *ps_dims = ps_of("dims");
    ID3D11SamplerState *point = sampler(D3D11_FILTER_MIN_MAG_MIP_POINT, 0), *linear = sampler(D3D11_FILTER_MIN_MAG_POINT_MIP_LINEAR, 0);
    ID3D11SamplerState *linear1 = sampler(D3D11_FILTER_MIN_MAG_POINT_MIP_LINEAR, 1.25f);

    /* views: all mips, and from mip 1 */
    ID3D11ShaderResourceView *all = NULL, *from1 = NULL;
    ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)tex, NULL, &all);
    D3D11_SHADER_RESOURCE_VIEW_DESC sd;
    memset(&sd, 0, sizeof sd);
    sd.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    sd.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
    sd.Texture2D.MostDetailedMip = 1;
    sd.Texture2D.MipLevels = 3;
    ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)tex, &sd, &from1);

    const float clamps[] = { 0, 0.5f, 1, 1.5f, 2, 3, 3.5f, 4, 10, -1 };
    const float lods[] = { 0, 0.25f, 1, 1.75f, 2.5f, 3, 5 };
    struct { const char *name; ID3D11ShaderResourceView *v; } views[] = { { "all mips", all }, { "from mip 1", from1 } };
    for (unsigned w = 0; w < 2; w++) {
        printf("view %s: red * 255 read under each clamp\n", views[w].name);
        ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 1, &views[w].v);
        for (unsigned c = 0; c < sizeof clamps / sizeof *clamps; c++) {
            ID3D11DeviceContext_SetResourceMinLOD(ctx, (ID3D11Resource *)tex, clamps[c]);
            float o[4];
            printf("  clamp %g\n", clamps[c]);
            struct { const char *name; ID3D11SamplerState *s; } ss[] = { { "point", point }, { "linear", linear }, { "linear MinLOD 1.25", linear1 } };
            for (unsigned k = 0; k < 3; k++) {
                ID3D11DeviceContext_PSSetSamplers(ctx, 0, 1, &ss[k].s);
                snprintf(line, sizeof line, "    %s level", ss[k].name);
                for (unsigned l = 0; l < sizeof lods / sizeof *lods; l++) {
                    run(ps_level, lods[l], 0, o);
                    add(" %g", floorf(o[0] * 255 * 100 + 0.5f) / 100);
                }
                run(ps_sample, 0, 0, o);
                add(", sample %g", floorf(o[0] * 255 * 100 + 0.5f) / 100);
                end_line();
            }
            snprintf(line, sizeof line, "    load");
            for (int m = 0; m < 4; m++) {
                run(ps_load, 0, m, o);
                add(" %g", floorf(o[0] * 255 * 100 + 0.5f) / 100);
            }
            end_line();
            snprintf(line, sizeof line, "    dims");
            for (int m = 0; m < 3; m++) {
                run(ps_dims, 0, m, o);
                add3(" %gx%g/%g", o[0], o[1], o[2]);
            }
            end_line();
        }
    }
    ID3D11DeviceContext_SetResourceMinLOD(ctx, (ID3D11Resource *)tex, 0);
    return 0;
}
