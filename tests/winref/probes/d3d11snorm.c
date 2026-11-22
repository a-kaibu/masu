/* SNORM render targets (R8G8B8A8_SNORM, R8G8_SNORM, R8_SNORM) as the output merger writes them:
 *  - a pixel shader's float output as the channel's byte: sweeps of 256 floats (consecutive
 *    bit patterns) around the ties of v * 127 = k + 0.5, around 1 and -1, and specials (NaN,
 *    infinities, -0, denormals, values past -1 and 1);
 *  - ClearRenderTargetView of the same specials and ties (and of R16G16_SNORM's and
 *    R8G8B8A8_UNORM's ties);
 *  - blending: every color blend factor pair (alpha ONE/ZERO) and every alpha factor pair
 *    (color ONE/ZERO) under each blend op, for three source/destination pairs (destinations
 *    with -128 and -127, sources past the range), dual-source factors (SRC1_*), BlendFactor
 *    past -1 and 1;
 *  - NaN and infinite sources (SV_Target0 and 1) blended;
 *  - which MIN / MAX blends with dual-source factors CreateBlendState takes;
 *  - two triangles of one draw blended over the same pixels; a write mask with blending;
 *  - an SNORM target beside an R8G8B8A8_UNORM one, both blended;
 *  - what was drawn read back by a shader (Texture2D Load) and by CopyResource;
 *  - 4x multisampled R8G8B8A8_SNORM: a half-covered triangle blended, every sample read
 *    through Texture2DMS, and ResolveSubresource; the resolve of samples each written by a
 *    per-sample shader (SV_SampleIndex) with its own bytes.
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11.h>
#include <d3dcompiler.h>

#define W 4
#define H 4
#define SW 256

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "cbuffer C : register(b0) { uint4 src; uint4 src1; uint4 sweep; };\n"
    "float4 vs_full(uint id : SV_VertexID) : SV_Position { return float4((id >> 1) * 4.0 - 1.0, (id & 1) * 4.0 - 1.0, 0.5, 1); }\n"
    "float4 vs_twice(uint id : SV_VertexID) : SV_Position { uint k = id % 3; return float4((k >> 1) * 4.0 - 1.0, (k & 1) * 4.0 - 1.0, 0.5, 1); }\n"
    "float4 vs_half(uint id : SV_VertexID) : SV_Position { return float4(id == 2 ? 1.0 : -1.0, id == 1 ? 1.0 : -1.0, 0.5, 1); }\n"
    "struct O2 { float4 a : SV_Target0; float4 b : SV_Target1; };\n"
    "O2 ps(float4 p : SV_Position) { O2 o; o.a = asfloat(src); o.b = asfloat(src1); return o; }\n"
    /* pixel x of the sweep: the float of bits sweep.x + x * sweep.y in every channel */
    "float4 ps_sweep(float4 p : SV_Position) : SV_Target { return asfloat(sweep.x + (uint)p.x * sweep.y).xxxx; }\n"
    "Texture2D<float4> t0 : register(t0);\n"
    "uint4 ps_load(float4 p : SV_Position) : SV_Target { return asuint(t0.Load(int3(p.xy, 0))); }\n"
    /* sample (x % 4) of pixel (x / 4, y) */
    "Texture2DMS<float4> m0 : register(t0);\n"
    "uint4 ps_ms(float4 p : SV_Position) : SV_Target { uint x = (uint)p.x; return asuint(m0.Load(int2(x / 4, (int)p.y), x % 4)); }\n"
    /* per sample: byte ((pixel * 4 + sample) * k + seed) % 255 - 127 of each channel (k 37 53 71 97,
     * seed sweep.x), as that byte / 127 */
    "float4 ps_samples(float4 p : SV_Position, uint s : SV_SampleIndex) : SV_Target {\n"
    "  uint i = ((uint)p.y * 4 + (uint)p.x) * 4 + s;\n"
    "  int4 b = (int4)((i * uint4(37, 53, 71, 97) + sweep.x) % 255) - 127;\n"
    "  return (float4)b / 127.0; }\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "snorm", NULL, NULL, entry, target, 0, 0, &b, &err);
    if (FAILED(hr)) printf("D3DCompile %s failed %#lx %s\n", entry, hr, err ? (char *)ID3D10Blob_GetBufferPointer(err) : "");
    if (err) ID3D10Blob_Release(err);
    return b;
}

static ID3D11Buffer *cb;
static UINT cbd[12];

static void upload(void) { ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)cb, 0, NULL, cbd, 0, 0); }

static UINT fbits(float f) {
    UINT u;
    memcpy(&u, &f, 4);
    return u;
}

struct target {
    ID3D11Texture2D *t, *staging;
    ID3D11RenderTargetView *v;
    ID3D11ShaderResourceView *srv;
    UINT bpp, w, h;
};

static int target(struct target *t, DXGI_FORMAT f, UINT bpp, UINT w, UINT h) {
    D3D11_TEXTURE2D_DESC td = { w, h, 1, 1, f, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE, 0, 0 };
    t->bpp = bpp;
    t->w = w;
    t->h = h;
    HRESULT hr = ID3D11Device_CreateTexture2D(dev, &td, NULL, &t->t);
    if (FAILED(hr)) { printf("CreateTexture2D %u failed %#lx\n", f, hr); return 0; }
    td.Usage = D3D11_USAGE_STAGING;
    td.BindFlags = 0;
    td.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    ID3D11Device_CreateTexture2D(dev, &td, NULL, &t->staging);
    ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)t->t, NULL, &t->v);
    ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)t->t, NULL, &t->srv);
    return 1;
}

static void fill(struct target *t, UINT px) {
    unsigned char all[SW * 4 * H];
    for (UINT i = 0; i < t->w * t->h; i++) memcpy(all + i * t->bpp, &px, t->bpp);
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)t->t, 0, NULL, all, t->w * t->bpp, 0);
}

/* texel (x, y) as one hex word (its bytes, little-endian), and whether every texel is it */
static void show(struct target *t, int x, int y, int check) {
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)t->staging, (ID3D11Resource *)t->t);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)t->staging, 0, D3D11_MAP_READ, 0, &m))) { printf(" map failed"); return; }
    const unsigned char *p = (const unsigned char *)m.pData + y * m.RowPitch + x * t->bpp;
    UINT v = 0;
    memcpy(&v, p, t->bpp);
    printf(" %0*x", (int)t->bpp * 2, v);
    if (check) {
        for (UINT yy = 0; yy < t->h; yy++)
            for (UINT xx = 0; xx < t->w; xx++)
                if (memcmp((const unsigned char *)m.pData + yy * m.RowPitch + xx * t->bpp, p, t->bpp)) { printf("(%u,%u differs)", xx, yy); yy = t->h; break; }
    }
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)t->staging, 0);
}

/* the first row's bytes as hex */
static void row(struct target *t) {
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)t->staging, (ID3D11Resource *)t->t);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)t->staging, 0, D3D11_MAP_READ, 0, &m))) { printf(" map failed"); return; }
    printf(" ");
    for (UINT k = 0; k < t->w * t->bpp; k++) printf("%02x", ((const unsigned char *)m.pData)[k]);
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)t->staging, 0);
}

static ID3D11BlendState *blend2(BOOL on, int sc, int dc, int oc, int sa, int da, int oa, UINT mask, BOOL independent) {
    D3D11_BLEND_DESC b;
    memset(&b, 0, sizeof b);
    b.IndependentBlendEnable = independent;
    for (int i = 0; i < 8; i++) {
        b.RenderTarget[i].BlendEnable = on;
        b.RenderTarget[i].SrcBlend = (D3D11_BLEND)sc;
        b.RenderTarget[i].DestBlend = (D3D11_BLEND)dc;
        b.RenderTarget[i].BlendOp = (D3D11_BLEND_OP)oc;
        b.RenderTarget[i].SrcBlendAlpha = (D3D11_BLEND)sa;
        b.RenderTarget[i].DestBlendAlpha = (D3D11_BLEND)da;
        b.RenderTarget[i].BlendOpAlpha = (D3D11_BLEND_OP)oa;
        b.RenderTarget[i].RenderTargetWriteMask = (UINT8)mask;
    }
    ID3D11BlendState *s = NULL;
    HRESULT hr = ID3D11Device_CreateBlendState(dev, &b, &s);
    if (FAILED(hr)) printf(" CreateBlendState failed %#lx", hr);
    return s;
}

/* a color factor's alpha twin (*_COLOR as *_ALPHA) */
static int alpha_of(int f) {
    switch (f) {
    case 3: return 5;
    case 4: return 6;
    case 9: return 7;
    case 10: return 8;
    case 16: return 18;
    case 17: return 19;
    default: return f;
    }
}

/* whether CreateBlendState takes the blend (o) or refuses it (x) */
static char takes(int sc, int dc, int oc, int sa, int da, int oa) {
    D3D11_BLEND_DESC b;
    memset(&b, 0, sizeof b);
    b.RenderTarget[0].BlendEnable = TRUE;
    b.RenderTarget[0].SrcBlend = (D3D11_BLEND)sc;
    b.RenderTarget[0].DestBlend = (D3D11_BLEND)dc;
    b.RenderTarget[0].BlendOp = (D3D11_BLEND_OP)oc;
    b.RenderTarget[0].SrcBlendAlpha = (D3D11_BLEND)sa;
    b.RenderTarget[0].DestBlendAlpha = (D3D11_BLEND)da;
    b.RenderTarget[0].BlendOpAlpha = (D3D11_BLEND_OP)oa;
    b.RenderTarget[0].RenderTargetWriteMask = 0xf;
    ID3D11BlendState *s = NULL;
    HRESULT hr = ID3D11Device_CreateBlendState(dev, &b, &s);
    if (s) ID3D11BlendState_Release(s);
    return SUCCEEDED(hr) ? 'o' : 'x';
}

static const int CF[17] = { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14, 15, 16, 17, 18, 19 };
static const char *CFN[17] = { "zero", "one", "src_color", "inv_src_color", "src_alpha", "inv_src_alpha", "dest_alpha", "inv_dest_alpha", "dest_color", "inv_dest_color", "src_alpha_sat", "blend_factor", "inv_blend_factor", "src1_color", "inv_src1_color", "src1_alpha", "inv_src1_alpha" };
/* (the factors an alpha blend takes: no *_COLOR) */
static const int AF[11] = { 1, 2, 5, 6, 7, 8, 11, 14, 15, 18, 19 };
static const char *AFN[11] = { "zero", "one", "src_alpha", "inv_src_alpha", "dest_alpha", "inv_dest_alpha", "src_alpha_sat", "blend_factor", "inv_blend_factor", "src1_alpha", "inv_src1_alpha" };
static const char *OPN[5] = { "add", "subtract", "rev_subtract", "min", "max" };

int main(void) {
    D3D_FEATURE_LEVEL fl;
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx)) &&
        FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx))) {
        puts("skip no device");
        return 0;
    }
    if (fl < D3D_FEATURE_LEVEL_10_0) { puts("skip feature level below 10_0"); return 0; }
    static const char *names[] = { "vs_full", "vs_twice", "vs_half", "ps", "ps_sweep", "ps_load", "ps_ms", "ps_samples" };
    static const char *kinds[] = { "vs_4_0", "vs_4_0", "vs_4_0", "ps_4_0", "ps_4_0", "ps_4_0", "ps_4_1", "ps_4_1" };
    ID3DBlob *bl[8];
    for (int i = 0; i < 8; i++)
        if (!(bl[i] = compile(names[i], kinds[i]))) return 0;
    ID3D11VertexShader *vs_full, *vs_twice, *vs_half;
    ID3D11PixelShader *ps, *ps_sweep, *ps_load, *ps_ms, *ps_samples;
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[0]), ID3D10Blob_GetBufferSize(bl[0]), NULL, &vs_full);
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[1]), ID3D10Blob_GetBufferSize(bl[1]), NULL, &vs_twice);
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[2]), ID3D10Blob_GetBufferSize(bl[2]), NULL, &vs_half);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[3]), ID3D10Blob_GetBufferSize(bl[3]), NULL, &ps);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[4]), ID3D10Blob_GetBufferSize(bl[4]), NULL, &ps_sweep);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[5]), ID3D10Blob_GetBufferSize(bl[5]), NULL, &ps_load);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[6]), ID3D10Blob_GetBufferSize(bl[6]), NULL, &ps_ms);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[7]), ID3D10Blob_GetBufferSize(bl[7]), NULL, &ps_samples);
    {
        D3D11_BUFFER_DESC bd = { 48, D3D11_USAGE_DEFAULT, D3D11_BIND_CONSTANT_BUFFER, 0, 0, 0 };
        ID3D11Device_CreateBuffer(dev, &bd, NULL, &cb);
    }
    ID3D11DeviceContext_PSSetConstantBuffers(ctx, 0, 1, &cb);
    D3D11_VIEWPORT vp = { 0, 0, W, H, 0, 1 };
    ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    ID3D11DeviceContext_VSSetShader(ctx, vs_full, NULL, 0);
    float zero[4] = { 0, 0, 0, 0 };

    puts("float outputs as bytes: sweeps of 256 consecutive floats (bits from, step) on R8_SNORM, R8G8_SNORM, R8G8B8A8_SNORM");
    {
        static const DXGI_FORMAT sf[3] = { DXGI_FORMAT_R8_SNORM, DXGI_FORMAT_R8G8_SNORM, DXGI_FORMAT_R8G8B8A8_SNORM };
        static const UINT sb[3] = { 1, 2, 4 };
        static const char *sn[3] = { "r8", "rg8", "rgba8" };
        struct target st[3];
        for (int f = 0; f < 3; f++)
            if (!target(&st[f], sf[f], sb[f], SW, 1)) return 0;
        D3D11_VIEWPORT sv = { 0, 0, SW, 1, 0, 1 };
        ID3D11DeviceContext_RSSetViewports(ctx, 1, &sv);
        ID3D11DeviceContext_PSSetShader(ctx, ps_sweep, NULL, 0);
        ID3D11DeviceContext_OMSetBlendState(ctx, NULL, zero, 0xffffffff);
        /* the ties (k + 0.5) / 127, both signs, and 1, -1 */
        static const double centre[] = { 0.5, 1.5, 2.5, 3.5, 4.5, 31.5, 63.5, 64.5, 95.5, 125.5, 126.5, -0.5, -1.5, -2.5, -63.5, -126.5, 127, -127, -128 };
        for (int i = 0; i < (int)(sizeof centre / sizeof centre[0]); i++) {
            UINT c = fbits((float)(centre[i] / 127.0));
            cbd[8] = c - 128;
            cbd[9] = 1;
            upload();
            for (int f = 0; f < 3; f++) {
                if (f != 0 && i != 0) continue;
                ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &st[f].v, NULL);
                ID3D11DeviceContext_Draw(ctx, 3, 0);
                printf("  %s %08x 1:", sn[f], c - 128);
                row(&st[f]);
                puts("");
            }
        }
        /* wide strides: every exponent of both signs (NaNs, infinities, denormals among them) */
        static const UINT wide[][2] = { { 0, 0x00800000 }, { 0x80000000, 0x00800000 }, { 0x7f800000, 1 }, { 0xff800000, 1 }, { 0, 1 }, { 0x80000000, 1 }, { 0x3c000000, 0x00020000 }, { 0xbc000000, 0x00020000 } };
        for (int i = 0; i < (int)(sizeof wide / sizeof wide[0]); i++) {
            cbd[8] = wide[i][0];
            cbd[9] = wide[i][1];
            upload();
            ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &st[0].v, NULL);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            printf("  r8 %08x %x:", wide[i][0], wide[i][1]);
            row(&st[0]);
            puts("");
        }
        ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
    }

    struct target t, u;
    if (!target(&t, DXGI_FORMAT_R8G8B8A8_SNORM, 4, W, H)) return 0;
    if (!target(&u, DXGI_FORMAT_R8G8B8A8_UNORM, 4, W, H)) return 0;

    puts("ClearRenderTargetView of R8G8B8A8_SNORM");
    {
        static const float cv[][4] = {
            { 0.25f, 0.5f, 1.0f, 0.75f }, { -1.0f, -2.0f, 2.0f, -0.0f }, { 0.5f / 127, 1.5f / 127, 2.5f / 127, -2.5f / 127 },
            { 63.5f / 127, -63.5f / 127, 126.5f / 127, -126.5f / 127 }, { 1e-30f, -1e-30f, 1.0f / 254, -1.0f / 254 },
        };
        for (int i = 0; i < 5; i++) {
            ID3D11DeviceContext_ClearRenderTargetView(ctx, t.v, cv[i]);
            printf("  %08x %08x %08x %08x:", fbits(cv[i][0]), fbits(cv[i][1]), fbits(cv[i][2]), fbits(cv[i][3]));
            show(&t, 1, 1, 1);
            puts("");
        }
        UINT nan = 0x7fc00000, inf = 0x7f800000, ninf = 0xff800000, snan = 0x7f800001;
        float sp[4];
        memcpy(&sp[0], &nan, 4);
        memcpy(&sp[1], &inf, 4);
        memcpy(&sp[2], &ninf, 4);
        memcpy(&sp[3], &snan, 4);
        ID3D11DeviceContext_ClearRenderTargetView(ctx, t.v, sp);
        printf("  nan inf -inf snan:");
        show(&t, 1, 1, 1);
        puts("");
    }
    puts("ClearRenderTargetView of R16G16_SNORM and R8G8B8A8_UNORM: halves of a step (the first pair each, as bits)");
    {
        struct target s16, u8;
        if (!target(&s16, DXGI_FORMAT_R16G16_SNORM, 4, W, H)) return 0;
        if (!target(&u8, DXGI_FORMAT_R8G8B8A8_UNORM, 4, W, H)) return 0;
        static const float c16[][4] = { { 0.5f / 32767, 1.5f / 32767, 0, 0 }, { 2.5f / 32767, -2.5f / 32767, 0, 0 }, { 16383.5f / 32767, -16383.5f / 32767, 0, 0 } };
        static const float cu[][4] = { { 0.5f / 255, 1.5f / 255, 2.5f / 255, 3.5f / 255 }, { 126.5f / 255, 127.5f / 255, 128.5f / 255, 254.5f / 255 } };
        for (int i = 0; i < 3; i++) {
            ID3D11DeviceContext_ClearRenderTargetView(ctx, s16.v, c16[i]);
            printf("  r16g16_snorm %08x %08x:", fbits(c16[i][0]), fbits(c16[i][1]));
            show(&s16, 1, 1, 1);
            puts("");
        }
        for (int i = 0; i < 2; i++) {
            ID3D11DeviceContext_ClearRenderTargetView(ctx, u8.v, cu[i]);
            printf("  rgba8_unorm %08x %08x %08x %08x:", fbits(cu[i][0]), fbits(cu[i][1]), fbits(cu[i][2]), fbits(cu[i][3]));
            show(&u8, 1, 1, 1);
            puts("");
        }
    }

    /* (source, second source, destination) sets */
    static const float srcs[3][4] = { { 0.25f, -0.5f, 0.75f, 0.6f }, { 1.5f, -2.0f, -0.3f, -0.4f }, { 0.1f, 0.9f, -0.95f, 1.0f } };
    static const float src1s[3][4] = { { 0.5f, -0.25f, 1.25f, -0.7f }, { -0.6f, 0.35f, 2.0f, 0.2f }, { 0.05f, -1.5f, 0.8f, -0.9f } };
    static const UINT dsts[3] = { 0x3081c040, 0xd0207f80, 0x7f7efb05 };
    static const float bf[4] = { 0.3f, -0.5f, 1.5f, 0.75f };
    ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    for (int s = 0; s < 3; s++) {
        printf("blend set %d: src %g %g %g %g src1 %g %g %g %g dst %08x factor 0.3 -0.5 1.5 0.75\n", s, srcs[s][0], srcs[s][1], srcs[s][2], srcs[s][3], src1s[s][0], src1s[s][1], src1s[s][2], src1s[s][3], dsts[s]);
        for (int k = 0; k < 4; k++) {
            cbd[k] = fbits(srcs[s][k]);
            cbd[4 + k] = fbits(src1s[s][k]);
        }
        upload();
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t.v, NULL);
        puts(" color (alpha one zero add): op src: each dest factor");
        for (int op = 1; op <= 5; op++) {
            for (int a = 0; a < 17; a++) {
                printf("  %s %s:", OPN[op - 1], CFN[a]);
                for (int b = 0; b < 17; b++) {
                    fill(&t, dsts[s]);
                    ID3D11BlendState *st = blend2(TRUE, CF[a], CF[b], op, 2, 1, 1, 0xf, FALSE);
                    ID3D11DeviceContext_OMSetBlendState(ctx, st, bf, 0xffffffff);
                    ID3D11DeviceContext_Draw(ctx, 3, 0);
                    show(&t, 2, 1, 0);
                    if (st) ID3D11BlendState_Release(st);
                }
                puts("");
                /* (min and max take no factors) */
                if (op >= 4) break;
            }
        }
        puts(" alpha (color one zero add): op src: each dest factor");
        for (int op = 1; op <= 5; op++) {
            for (int a = 0; a < 11; a++) {
                printf("  %s %s:", OPN[op - 1], AFN[a]);
                for (int b = 0; b < 11; b++) {
                    fill(&t, dsts[s]);
                    ID3D11BlendState *st = blend2(TRUE, 2, 1, 1, AF[a], AF[b], op, 0xf, FALSE);
                    ID3D11DeviceContext_OMSetBlendState(ctx, st, bf, 0xffffffff);
                    ID3D11DeviceContext_Draw(ctx, 3, 0);
                    show(&t, 2, 1, 0);
                    if (st) ID3D11BlendState_Release(st);
                }
                puts("");
                if (op >= 4) break;
            }
        }
    }

    puts("NaN and infinite sources blended over 3081c040 (factor 0.3 -0.5 1.5 0.75): src, src1 as bits, then each blend (src dst op, alpha the same)");
    {
        static const UINT ns[4][8] = {
            { 0x7fc00000, 0x3f000000, 0xffc00000, 0x7f800001, 0x3f000000, 0x3e800000, 0x3f000000, 0x3f000000 },
            { 0x7f800000, 0xff800000, 0x7fc00000, 0x3f000000, 0x7fc00000, 0xffc00000, 0x7f800000, 0x7fc00000 },
            { 0x3f000000, 0x7fc00000, 0x3e800000, 0x7fc00000, 0x3e800000, 0x3f000000, 0xff800000, 0x3f000000 },
            { 0xffc00000, 0x7f800000, 0xff800000, 0x3e800000, 0x7fc00000, 0x7fc00000, 0x7fc00000, 0x7fc00000 },
        };
        static const int bl[9][3] = { { 2, 1, 1 }, { 2, 2, 1 }, { 5, 6, 1 }, { 9, 1, 1 }, { 16, 1, 1 }, { 1, 18, 1 }, { 2, 2, 2 }, { 2, 1, 4 }, { 2, 1, 5 } };
        for (int i = 0; i < 4; i++) {
            for (int k = 0; k < 8; k++) cbd[k] = ns[i][k];
            upload();
            printf("  %08x %08x %08x %08x / %08x %08x %08x %08x:", ns[i][0], ns[i][1], ns[i][2], ns[i][3], ns[i][4], ns[i][5], ns[i][6], ns[i][7]);
            for (int j = 0; j < 9; j++) {
                int a0 = alpha_of(bl[j][0]), a1 = alpha_of(bl[j][1]);
                fill(&t, 0x3081c040);
                ID3D11BlendState *st = blend2(TRUE, bl[j][0], bl[j][1], bl[j][2], a0, a1, bl[j][2], 0xf, FALSE);
                ID3D11DeviceContext_OMSetBlendState(ctx, st, bf, 0xffffffff);
                ID3D11DeviceContext_Draw(ctx, 3, 0);
                show(&t, 2, 1, 0);
                if (st) ID3D11BlendState_Release(st);
            }
            puts("");
        }
    }

    puts("MIN / MAX with every factor pair: what CreateBlendState takes (o) or refuses (x), each src factor's row of dest factors");
    for (int op = 4; op <= 5; op++) {
        for (int a = 0; a < 17; a++) {
            printf("  %s color %s: ", OPN[op - 1], CFN[a]);
            for (int b = 0; b < 17; b++) putchar(takes(CF[a], CF[b], op, 2, 1, 1));
            puts("");
        }
        for (int a = 0; a < 11; a++) {
            printf("  %s alpha %s: ", OPN[op - 1], AFN[a]);
            for (int b = 0; b < 11; b++) putchar(takes(2, 1, 1, AF[a], AF[b], op));
            puts("");
        }
    }

    puts("two triangles of one draw blended (src 0.3 -0.3 0.6 0.5 over 0): one one add, src_alpha inv_src_alpha add, dest_color src_color add");
    {
        float s3[4] = { 0.3f, -0.3f, 0.6f, 0.5f };
        for (int k = 0; k < 4; k++) cbd[k] = fbits(s3[k]);
        upload();
        ID3D11DeviceContext_VSSetShader(ctx, vs_twice, NULL, 0);
        static const int pairs[3][2] = { { 2, 2 }, { 5, 6 }, { 9, 3 } };
        for (int i = 0; i < 3; i++) {
            fill(&t, 0x20e0a040);
            ID3D11BlendState *st = blend2(TRUE, pairs[i][0], pairs[i][1], 1, pairs[i][0] == 9 ? 7 : pairs[i][0], pairs[i][1] == 3 ? 5 : pairs[i][1], 1, 0xf, FALSE);
            ID3D11DeviceContext_OMSetBlendState(ctx, st, zero, 0xffffffff);
            ID3D11DeviceContext_Draw(ctx, 6, 0);
            printf("  %d:", i);
            show(&t, 3, 2, 1);
            puts("");
            if (st) ID3D11BlendState_Release(st);
        }
        ID3D11DeviceContext_VSSetShader(ctx, vs_full, NULL, 0);
    }

    puts("write masks with src_alpha inv_src_alpha add (blend set 0)");
    {
        for (int k = 0; k < 4; k++) cbd[k] = fbits(srcs[0][k]);
        upload();
        static const UINT masks[] = { 0, 1, 2, 4, 8, 5, 0xa, 0xe };
        for (int i = 0; i < 8; i++) {
            fill(&t, dsts[0]);
            ID3D11BlendState *st = blend2(TRUE, 5, 6, 1, 5, 6, 1, masks[i], FALSE);
            ID3D11DeviceContext_OMSetBlendState(ctx, st, zero, 0xffffffff);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            printf("  mask %x:", masks[i]);
            show(&t, 0, 3, 1);
            puts("");
            if (st) ID3D11BlendState_Release(st);
        }
    }

    puts("an SNORM and an UNORM target together (src_alpha inv_src_alpha add; blend set 0, second source to the UNORM one)");
    {
        for (int k = 0; k < 4; k++) {
            cbd[k] = fbits(srcs[0][k]);
            cbd[4 + k] = fbits(src1s[0][k]);
        }
        upload();
        ID3D11RenderTargetView *two[2] = { t.v, u.v };
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 2, two, NULL);
        fill(&t, dsts[0]);
        fill(&u, 0x80402010);
        ID3D11BlendState *st = blend2(TRUE, 5, 6, 1, 5, 6, 1, 0xf, FALSE);
        ID3D11DeviceContext_OMSetBlendState(ctx, st, zero, 0xffffffff);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        printf("  snorm:");
        show(&t, 1, 2, 1);
        printf(" unorm:");
        show(&u, 1, 2, 1);
        puts("");
        if (st) ID3D11BlendState_Release(st);
        /* the UNORM one first, the SNORM one second */
        ID3D11RenderTargetView *rev[2] = { u.v, t.v };
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 2, rev, NULL);
        fill(&t, dsts[0]);
        fill(&u, 0x80402010);
        st = blend2(TRUE, 2, 2, 1, 2, 2, 1, 0xf, FALSE);
        ID3D11DeviceContext_OMSetBlendState(ctx, st, zero, 0xffffffff);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        printf("  one one add, unorm 0 snorm 1: unorm:");
        show(&u, 1, 2, 1);
        printf(" snorm:");
        show(&t, 1, 2, 1);
        puts("");
        if (st) ID3D11BlendState_Release(st);
    }

    puts("what was drawn, read by a shader (Load, as float bits) and by CopyResource");
    {
        for (int k = 0; k < 4; k++) cbd[k] = fbits(srcs[1][k]);
        upload();
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t.v, NULL);
        fill(&t, dsts[1]);
        ID3D11BlendState *st = blend2(TRUE, 2, 2, 1, 2, 2, 1, 0xf, FALSE);
        ID3D11DeviceContext_OMSetBlendState(ctx, st, zero, 0xffffffff);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        if (st) ID3D11BlendState_Release(st);
        printf("  copy:");
        show(&t, 0, 0, 1);
        puts("");
        D3D11_TEXTURE2D_DESC fd = { W, H, 1, 1, DXGI_FORMAT_R32G32B32A32_UINT, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
        ID3D11Texture2D *ft, *fs;
        ID3D11RenderTargetView *fv;
        ID3D11Device_CreateTexture2D(dev, &fd, NULL, &ft);
        fd.Usage = D3D11_USAGE_STAGING;
        fd.BindFlags = 0;
        fd.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
        ID3D11Device_CreateTexture2D(dev, &fd, NULL, &fs);
        ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)ft, NULL, &fv);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &fv, NULL);
        ID3D11DeviceContext_OMSetBlendState(ctx, NULL, zero, 0xffffffff);
        ID3D11DeviceContext_PSSetShader(ctx, ps_load, NULL, 0);
        ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 1, &t.srv);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        ID3D11ShaderResourceView *none = NULL;
        ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 1, &none);
        ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)fs, (ID3D11Resource *)ft);
        D3D11_MAPPED_SUBRESOURCE m;
        if (SUCCEEDED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)fs, 0, D3D11_MAP_READ, 0, &m))) {
            const UINT *p = (const UINT *)((const unsigned char *)m.pData + m.RowPitch);
            printf("  load: %08x %08x %08x %08x\n", p[4], p[5], p[6], p[7]);
            ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)fs, 0);
        }
        /* a draw after the shader read it: blended over what the first left */
        ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t.v, NULL);
        st = blend2(TRUE, 2, 2, 1, 2, 2, 1, 0xf, FALSE);
        ID3D11DeviceContext_OMSetBlendState(ctx, st, zero, 0xffffffff);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        if (st) ID3D11BlendState_Release(st);
        printf("  again:");
        show(&t, 0, 0, 1);
        puts("");
        ID3D11Texture2D_Release(ft);
        ID3D11Texture2D_Release(fs);
        ID3D11RenderTargetView_Release(fv);
    }

    puts("4x multisampled R8G8B8A8_SNORM: a half-covered triangle blended (src_alpha inv_src_alpha add, set 0), each pixel's 4 samples as float bits, then ResolveSubresource");
    {
        UINT q = 0;
        ID3D11Device_CheckMultisampleQualityLevels(dev, DXGI_FORMAT_R8G8B8A8_SNORM, 4, &q);
        if (!q) { puts("  no 4x R8G8B8A8_SNORM"); puts("done"); return 0; }
        D3D11_TEXTURE2D_DESC md = { W, H, 1, 1, DXGI_FORMAT_R8G8B8A8_SNORM, { 4, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE, 0, 0 };
        ID3D11Texture2D *mt;
        ID3D11RenderTargetView *mv;
        ID3D11ShaderResourceView *ms;
        ID3D11Device_CreateTexture2D(dev, &md, NULL, &mt);
        ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)mt, NULL, &mv);
        ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)mt, NULL, &ms);
        static const float dc[4] = { 64.0f / 127, -64.0f / 127, -1.0f, 48.0f / 127 };
        ID3D11DeviceContext_ClearRenderTargetView(ctx, mv, dc);
        for (int k = 0; k < 4; k++) cbd[k] = fbits(srcs[0][k]);
        upload();
        ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
        ID3D11DeviceContext_VSSetShader(ctx, vs_half, NULL, 0);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &mv, NULL);
        ID3D11BlendState *st = blend2(TRUE, 5, 6, 1, 5, 6, 1, 0xf, FALSE);
        ID3D11DeviceContext_OMSetBlendState(ctx, st, zero, 0xffffffff);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        if (st) ID3D11BlendState_Release(st);
        D3D11_TEXTURE2D_DESC fd = { W * 4, H, 1, 1, DXGI_FORMAT_R32G32B32A32_UINT, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
        ID3D11Texture2D *ft, *fs;
        ID3D11RenderTargetView *fv;
        ID3D11Device_CreateTexture2D(dev, &fd, NULL, &ft);
        fd.Usage = D3D11_USAGE_STAGING;
        fd.BindFlags = 0;
        fd.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
        ID3D11Device_CreateTexture2D(dev, &fd, NULL, &fs);
        ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)ft, NULL, &fv);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &fv, NULL);
        ID3D11DeviceContext_OMSetBlendState(ctx, NULL, zero, 0xffffffff);
        D3D11_VIEWPORT wv = { 0, 0, W * 4, H, 0, 1 };
        ID3D11DeviceContext_RSSetViewports(ctx, 1, &wv);
        ID3D11DeviceContext_VSSetShader(ctx, vs_full, NULL, 0);
        ID3D11DeviceContext_PSSetShader(ctx, ps_ms, NULL, 0);
        ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 1, &ms);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        ID3D11ShaderResourceView *none = NULL;
        ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 1, &none);
        ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
        ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)fs, (ID3D11Resource *)ft);
        D3D11_MAPPED_SUBRESOURCE m;
        if (SUCCEEDED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)fs, 0, D3D11_MAP_READ, 0, &m))) {
            for (int y = 0; y < H; y++) {
                printf("  row %d:", y);
                const UINT *p = (const UINT *)((const unsigned char *)m.pData + y * m.RowPitch);
                for (int x = 0; x < W * 4; x++) printf(" %08x.%08x.%08x.%08x", p[x * 4], p[x * 4 + 1], p[x * 4 + 2], p[x * 4 + 3]);
                puts("");
            }
            ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)fs, 0);
        }
        ID3D11DeviceContext_ResolveSubresource(ctx, (ID3D11Resource *)t.t, 0, (ID3D11Resource *)mt, 0, DXGI_FORMAT_R8G8B8A8_SNORM);
        printf("  resolved:");
        for (int i = 0; i < W * H; i++) show(&t, i % W, i / W, 0);
        puts("");
        puts("  ResolveSubresource of samples each its own (ps_samples: each seed's pixels' resolved words)");
        ID3D11DeviceContext_VSSetShader(ctx, vs_full, NULL, 0);
        ID3D11DeviceContext_PSSetShader(ctx, ps_samples, NULL, 0);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &mv, NULL);
        for (UINT seed = 0; seed < 255; seed += 17) {
            cbd[8] = seed;
            upload();
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            ID3D11DeviceContext_ResolveSubresource(ctx, (ID3D11Resource *)t.t, 0, (ID3D11Resource *)mt, 0, DXGI_FORMAT_R8G8B8A8_SNORM);
            printf("  seed %u:", seed);
            for (int i = 0; i < W * H; i++) show(&t, i % W, i / W, 0);
            puts("");
        }
        ID3D11Texture2D_Release(ft);
        ID3D11Texture2D_Release(fs);
        ID3D11RenderTargetView_Release(fv);
        ID3D11Texture2D_Release(mt);
        ID3D11RenderTargetView_Release(mv);
        ID3D11ShaderResourceView_Release(ms);
    }
    puts("done");
    return 0;
}
