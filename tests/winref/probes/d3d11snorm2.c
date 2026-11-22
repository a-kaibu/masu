/* SNORM render targets past d3d11snorm's 8-bit ones (R16G16B16A16_SNORM, R16G16_SNORM, R16_SNORM;
 * a multisampled R8G8B8A8_TYPELESS texture with SNORM views; ClearView rectangles):
 *  - a pixel shader's float output as R16_SNORM's word: sweeps of 256 floats (consecutive bit
 *    patterns) around the ties of v * 32767 = k + 0.5, around 1 and -1, and specials;
 *  - R16_SNORM's words read by a shader (Texture2D Load, as float bits): 1024 of them, -32768
 *    among them;
 *  - blending on R16G16B16A16_SNORM: every color blend factor pair (alpha ONE/ZERO) and every
 *    alpha factor pair (color ONE/ZERO) under each blend op, for three source/destination
 *    pairs (destinations with -32768 and -32767, sources past the range), and NaN and
 *    infinite sources;
 *  - 4x multisampled R16G16B16A16_SNORM: a half-covered triangle blended, every sample read
 *    through Texture2DMS, and the resolve of samples each written by a per-sample shader;
 *  - 4x multisampled R8G8B8A8_TYPELESS through R8G8B8A8_SNORM views: cleared, a half-covered
 *    triangle blended, every sample read, ResolveSubresource as R8G8B8A8_SNORM, and through
 *    an R8G8B8A8_UNORM view drawn and read;
 *  - ID3D11DeviceContext1::ClearView with rectangles on R8G8B8A8_SNORM (single sample and 4x),
 *    on the typeless 4x texture's SNORM view, and on 4x R16G16B16A16_SNORM.
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11_1.h>
#include <d3dcompiler.h>

#define W 4
#define H 4
#define SW 256

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;
static ID3D11DeviceContext1 *ctx1;

static const char *SRC =
    "cbuffer C : register(b0) { uint4 src; uint4 src1; uint4 sweep; };\n"
    "float4 vs_full(uint id : SV_VertexID) : SV_Position { return float4((id >> 1) * 4.0 - 1.0, (id & 1) * 4.0 - 1.0, 0.5, 1); }\n"
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
    /* per sample: ((pixel * 4 + sample) * k + seed) % 65535 - 32767 of each channel (k 9973 7919
     * 6151 4099, seed sweep.x), as that value / 32767 */
    "float4 ps_samples(float4 p : SV_Position, uint s : SV_SampleIndex) : SV_Target {\n"
    "  uint i = ((uint)p.y * 4 + (uint)p.x) * 4 + s;\n"
    "  int4 b = (int4)((i * uint4(9973, 7919, 6151, 4099) + sweep.x) % 65535) - 32767;\n"
    "  return (float4)b / 32767.0; }\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "snorm2", NULL, NULL, entry, target, 0, 0, &b, &err);
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

/* every texel the bpp bytes at px */
static void fill(struct target *t, const void *px) {
    static unsigned char all[SW * 8 * H];
    for (UINT i = 0; i < t->w * t->h; i++) memcpy(all + i * t->bpp, px, t->bpp);
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)t->t, 0, NULL, all, t->w * t->bpp, 0);
}

/* an 8-byte texel of four 16-bit words, the first lowest */
static unsigned long long w4(UINT a, UINT b, UINT c, UINT d) {
    return (unsigned long long)(a & 0xffff) | (unsigned long long)(b & 0xffff) << 16 | (unsigned long long)(c & 0xffff) << 32 | (unsigned long long)(d & 0xffff) << 48;
}

/* texel bytes as hex, the last byte first (a little-endian number) */
static void hexbytes(const unsigned char *p, UINT n) {
    printf(" ");
    for (UINT k = n; k > 0; k--) printf("%02x", p[k - 1]);
}

/* texel (x, y) as one hex number, and whether every texel is it */
static void show(struct target *t, int x, int y, int check) {
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)t->staging, (ID3D11Resource *)t->t);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)t->staging, 0, D3D11_MAP_READ, 0, &m))) { printf(" map failed"); return; }
    const unsigned char *p = (const unsigned char *)m.pData + y * m.RowPitch + x * t->bpp;
    hexbytes(p, t->bpp);
    if (check) {
        for (UINT yy = 0; yy < t->h; yy++)
            for (UINT xx = 0; xx < t->w; xx++)
                if (memcmp((const unsigned char *)m.pData + yy * m.RowPitch + xx * t->bpp, p, t->bpp)) { printf("(%u,%u differs)", xx, yy); yy = t->h; break; }
    }
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)t->staging, 0);
}

/* the first row's bytes as hex, in memory order */
static void row(struct target *t) {
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)t->staging, (ID3D11Resource *)t->t);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)t->staging, 0, D3D11_MAP_READ, 0, &m))) { printf(" map failed"); return; }
    printf(" ");
    for (UINT k = 0; k < t->w * t->bpp; k++) printf("%02x", ((const unsigned char *)m.pData)[k]);
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)t->staging, 0);
}

static ID3D11BlendState *blend2(BOOL on, int sc, int dc, int oc, int sa, int da, int oa, UINT mask) {
    D3D11_BLEND_DESC b;
    memset(&b, 0, sizeof b);
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

static const int CF[17] = { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14, 15, 16, 17, 18, 19 };
static const char *CFN[17] = { "zero", "one", "src_color", "inv_src_color", "src_alpha", "inv_src_alpha", "dest_alpha", "inv_dest_alpha", "dest_color", "inv_dest_color", "src_alpha_sat", "blend_factor", "inv_blend_factor", "src1_color", "inv_src1_color", "src1_alpha", "inv_src1_alpha" };
static const int AF[11] = { 1, 2, 5, 6, 7, 8, 11, 14, 15, 18, 19 };
static const char *AFN[11] = { "zero", "one", "src_alpha", "inv_src_alpha", "dest_alpha", "inv_dest_alpha", "src_alpha_sat", "blend_factor", "inv_blend_factor", "src1_alpha", "inv_src1_alpha" };
static const char *OPN[5] = { "add", "subtract", "rev_subtract", "min", "max" };

static ID3D11VertexShader *vs_full, *vs_half;
static ID3D11PixelShader *ps, *ps_sweep, *ps_load, *ps_ms, *ps_samples;
static D3D11_VIEWPORT vp = { 0, 0, W, H, 0, 1 };
static const float zero[4] = { 0, 0, 0, 0 };

/* a 4x multisampled texture of format f (its views of format v) */
struct mstex {
    ID3D11Texture2D *t;
    ID3D11RenderTargetView *v;
    ID3D11ShaderResourceView *srv;
};

static int mstex(struct mstex *m, DXGI_FORMAT f, DXGI_FORMAT v) {
    D3D11_TEXTURE2D_DESC md = { W, H, 1, 1, f, { 4, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE, 0, 0 };
    if (FAILED(ID3D11Device_CreateTexture2D(dev, &md, NULL, &m->t))) { printf("  CreateTexture2D 4x %u failed\n", f); return 0; }
    D3D11_RENDER_TARGET_VIEW_DESC rd;
    memset(&rd, 0, sizeof rd);
    rd.Format = v;
    rd.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE2DMS;
    D3D11_SHADER_RESOURCE_VIEW_DESC sd;
    memset(&sd, 0, sizeof sd);
    sd.Format = v;
    sd.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2DMS;
    if (FAILED(ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)m->t, &rd, &m->v))) { printf("  CreateRenderTargetView %u failed\n", v); return 0; }
    if (FAILED(ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)m->t, &sd, &m->srv))) { printf("  CreateShaderResourceView %u failed\n", v); return 0; }
    return 1;
}

/* every sample of srv (4x, W x H) read through Texture2DMS as float bits: a line a row */
static void ms_rows(ID3D11ShaderResourceView *srv) {
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
    ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 1, &srv);
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
    ID3D11Texture2D_Release(ft);
    ID3D11Texture2D_Release(fs);
    ID3D11RenderTargetView_Release(fv);
}

/* the half-covered triangle of source src (one blend for color and alpha) over what rtv holds */
static void half(ID3D11RenderTargetView *rtv, const float s[4], int sc, int dc, int op) {
    for (int k = 0; k < 4; k++) cbd[k] = fbits(s[k]);
    upload();
    ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    ID3D11DeviceContext_VSSetShader(ctx, vs_half, NULL, 0);
    ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &rtv, NULL);
    ID3D11BlendState *st = blend2(TRUE, sc, dc, op, alpha_of(sc), alpha_of(dc), op, 0xf);
    ID3D11DeviceContext_OMSetBlendState(ctx, st, zero, 0xffffffff);
    ID3D11DeviceContext_Draw(ctx, 3, 0);
    if (st) ID3D11BlendState_Release(st);
    ID3D11DeviceContext_VSSetShader(ctx, vs_full, NULL, 0);
}

/* ResolveSubresource of m as format f into r, each texel */
static void resolved(struct target *r, struct mstex *m, DXGI_FORMAT f) {
    ID3D11DeviceContext_ResolveSubresource(ctx, (ID3D11Resource *)r->t, 0, (ID3D11Resource *)m->t, 0, f);
    for (int i = 0; i < W * H; i++) show(r, i % W, i / W, 0);
    puts("");
}

int main(void) {
    D3D_FEATURE_LEVEL fl;
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx)) &&
        FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx))) {
        puts("skip no device");
        return 0;
    }
    if (fl < D3D_FEATURE_LEVEL_10_1) { puts("skip feature level below 10_1"); return 0; }
    if (FAILED(ID3D11DeviceContext_QueryInterface(ctx, &IID_ID3D11DeviceContext1, (void **)&ctx1))) ctx1 = NULL;
    static const char *names[] = { "vs_full", "vs_half", "ps", "ps_sweep", "ps_load", "ps_ms", "ps_samples" };
    static const char *kinds[] = { "vs_4_0", "vs_4_0", "ps_4_0", "ps_4_0", "ps_4_0", "ps_4_1", "ps_4_1" };
    ID3DBlob *bl[7];
    for (int i = 0; i < 7; i++)
        if (!(bl[i] = compile(names[i], kinds[i]))) return 0;
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[0]), ID3D10Blob_GetBufferSize(bl[0]), NULL, &vs_full);
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[1]), ID3D10Blob_GetBufferSize(bl[1]), NULL, &vs_half);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[2]), ID3D10Blob_GetBufferSize(bl[2]), NULL, &ps);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[3]), ID3D10Blob_GetBufferSize(bl[3]), NULL, &ps_sweep);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[4]), ID3D10Blob_GetBufferSize(bl[4]), NULL, &ps_load);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[5]), ID3D10Blob_GetBufferSize(bl[5]), NULL, &ps_ms);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[6]), ID3D10Blob_GetBufferSize(bl[6]), NULL, &ps_samples);
    {
        D3D11_BUFFER_DESC bd = { 48, D3D11_USAGE_DEFAULT, D3D11_BIND_CONSTANT_BUFFER, 0, 0, 0 };
        ID3D11Device_CreateBuffer(dev, &bd, NULL, &cb);
    }
    ID3D11DeviceContext_PSSetConstantBuffers(ctx, 0, 1, &cb);
    ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    ID3D11DeviceContext_VSSetShader(ctx, vs_full, NULL, 0);

    puts("float outputs as 16-bit words: sweeps of 256 consecutive floats (bits from, step) on R16_SNORM, R16G16_SNORM, R16G16B16A16_SNORM (bytes in memory order)");
    {
        static const DXGI_FORMAT sf[3] = { DXGI_FORMAT_R16_SNORM, DXGI_FORMAT_R16G16_SNORM, DXGI_FORMAT_R16G16B16A16_SNORM };
        static const UINT sb[3] = { 2, 4, 8 };
        static const char *sn[3] = { "r16", "rg16", "rgba16" };
        struct target st[3];
        for (int f = 0; f < 3; f++)
            if (!target(&st[f], sf[f], sb[f], SW, 1)) return 0;
        D3D11_VIEWPORT sv = { 0, 0, SW, 1, 0, 1 };
        ID3D11DeviceContext_RSSetViewports(ctx, 1, &sv);
        ID3D11DeviceContext_PSSetShader(ctx, ps_sweep, NULL, 0);
        ID3D11DeviceContext_OMSetBlendState(ctx, NULL, zero, 0xffffffff);
        static const double centre[] = { 0.5, 1.5, 2.5, 3.5, 100.5, 8191.5, 16383.5, 16384.5, 32765.5, 32766.5, -0.5, -1.5, -2.5, -16383.5, -32766.5, 32767, -32767, -32768 };
        for (int i = 0; i < (int)(sizeof centre / sizeof centre[0]); i++) {
            UINT c = fbits((float)(centre[i] / 32767.0));
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
        static const UINT wide[][2] = { { 0, 0x00800000 }, { 0x80000000, 0x00800000 }, { 0x7f800000, 1 }, { 0xff800000, 1 }, { 0, 1 }, { 0x80000000, 1 }, { 0x38000000, 0x00020000 }, { 0xb8000000, 0x00020000 } };
        for (int i = 0; i < (int)(sizeof wide / sizeof wide[0]); i++) {
            cbd[8] = wide[i][0];
            cbd[9] = wide[i][1];
            upload();
            ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &st[0].v, NULL);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            printf("  r16 %08x %x:", wide[i][0], wide[i][1]);
            row(&st[0]);
            puts("");
        }
        ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
    }

    puts("R16_SNORM words read by a shader (Load, as float bits): word, then its float, a line of 64 each");
    {
        D3D11_TEXTURE2D_DESC td = { SW, 4, 1, 1, DXGI_FORMAT_R16_SNORM, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_SHADER_RESOURCE, 0, 0 };
        static unsigned short words[SW * 4];
        for (UINT i = 0; i < SW * 4; i++) words[i] = (unsigned short)(i * 64 + ((i * 37) & 63));
        static const unsigned short special[] = { 0x8000, 0x8001, 0x7fff, 0x0000, 0x0001, 0xffff, 0x4000, 0xc000 };
        for (int i = 0; i < 8; i++) words[i] = special[i];
        D3D11_SUBRESOURCE_DATA init = { words, SW * 2, 0 };
        ID3D11Texture2D *lt;
        ID3D11ShaderResourceView *ls;
        ID3D11Device_CreateTexture2D(dev, &td, &init, &lt);
        ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)lt, NULL, &ls);
        D3D11_TEXTURE2D_DESC fd = { SW, 4, 1, 1, DXGI_FORMAT_R32G32B32A32_UINT, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
        ID3D11Texture2D *ft, *fs;
        ID3D11RenderTargetView *fv;
        ID3D11Device_CreateTexture2D(dev, &fd, NULL, &ft);
        fd.Usage = D3D11_USAGE_STAGING;
        fd.BindFlags = 0;
        fd.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
        ID3D11Device_CreateTexture2D(dev, &fd, NULL, &fs);
        ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)ft, NULL, &fv);
        D3D11_VIEWPORT lv = { 0, 0, SW, 4, 0, 1 };
        ID3D11DeviceContext_RSSetViewports(ctx, 1, &lv);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &fv, NULL);
        ID3D11DeviceContext_OMSetBlendState(ctx, NULL, zero, 0xffffffff);
        ID3D11DeviceContext_PSSetShader(ctx, ps_load, NULL, 0);
        ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 1, &ls);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        ID3D11ShaderResourceView *none = NULL;
        ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 1, &none);
        ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
        ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)fs, (ID3D11Resource *)ft);
        D3D11_MAPPED_SUBRESOURCE m;
        if (SUCCEEDED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)fs, 0, D3D11_MAP_READ, 0, &m))) {
            for (UINT i = 0; i < SW * 4; i++) {
                const UINT *p = (const UINT *)((const unsigned char *)m.pData + (i / SW) * m.RowPitch) + (i % SW) * 4;
                if (i % 64 == 0) printf("  load:");
                printf(" %04x=%08x", words[i], p[0]);
                if (i % 64 == 63) puts("");
            }
            ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)fs, 0);
        }
        ID3D11Texture2D_Release(ft);
        ID3D11Texture2D_Release(fs);
        ID3D11RenderTargetView_Release(fv);
        ID3D11ShaderResourceView_Release(ls);
        ID3D11Texture2D_Release(lt);
    }

    struct target t16;
    if (!target(&t16, DXGI_FORMAT_R16G16B16A16_SNORM, 8, W, H)) return 0;
    static const float srcs[3][4] = { { 0.25f, -0.5f, 0.75f, 0.6f }, { 1.5f, -2.0f, -0.3f, -0.4f }, { 0.1f, 0.9f, -0.95f, 1.0f } };
    static const float src1s[3][4] = { { 0.5f, -0.25f, 1.25f, -0.7f }, { -0.6f, 0.35f, 2.0f, 0.2f }, { 0.05f, -1.5f, 0.8f, -0.9f } };
    const unsigned long long dsts[3] = { w4(0x1234, 0x8000, 0xc001, 0x6000), w4(0x8001, 0x7fff, 0x0001, 0xfffe), w4(0x4000, 0x3fff, 0xa5a5, 0x5a5a) };
    static const float bf[4] = { 0.3f, -0.5f, 1.5f, 0.75f };
    ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    for (int s = 0; s < 3; s++) {
        printf("blend16 set %d: src %g %g %g %g src1 %g %g %g %g dst %08x%08x factor 0.3 -0.5 1.5 0.75\n", s, srcs[s][0], srcs[s][1], srcs[s][2], srcs[s][3], src1s[s][0], src1s[s][1], src1s[s][2], src1s[s][3], (UINT)(dsts[s] >> 32), (UINT)dsts[s]);
        for (int k = 0; k < 4; k++) {
            cbd[k] = fbits(srcs[s][k]);
            cbd[4 + k] = fbits(src1s[s][k]);
        }
        upload();
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t16.v, NULL);
        puts(" color (alpha one zero add): op src: each dest factor");
        for (int op = 1; op <= 5; op++) {
            for (int a = 0; a < 17; a++) {
                printf("  %s %s:", OPN[op - 1], CFN[a]);
                for (int b = 0; b < 17; b++) {
                    fill(&t16, &dsts[s]);
                    ID3D11BlendState *st = blend2(TRUE, CF[a], CF[b], op, 2, 1, 1, 0xf);
                    ID3D11DeviceContext_OMSetBlendState(ctx, st, bf, 0xffffffff);
                    ID3D11DeviceContext_Draw(ctx, 3, 0);
                    show(&t16, 2, 1, 0);
                    if (st) ID3D11BlendState_Release(st);
                }
                puts("");
                if (op >= 4) break;
            }
        }
        puts(" alpha (color one zero add): op src: each dest factor");
        for (int op = 1; op <= 5; op++) {
            for (int a = 0; a < 11; a++) {
                printf("  %s %s:", OPN[op - 1], AFN[a]);
                for (int b = 0; b < 11; b++) {
                    fill(&t16, &dsts[s]);
                    ID3D11BlendState *st = blend2(TRUE, 2, 1, 1, AF[a], AF[b], op, 0xf);
                    ID3D11DeviceContext_OMSetBlendState(ctx, st, bf, 0xffffffff);
                    ID3D11DeviceContext_Draw(ctx, 3, 0);
                    show(&t16, 2, 1, 0);
                    if (st) ID3D11BlendState_Release(st);
                }
                puts("");
                if (op >= 4) break;
            }
        }
    }

    puts("blend16 NaN and infinite sources over set 0's destination (factor 0.3 -0.5 1.5 0.75): src, src1 as bits, then each blend (src dst op, alpha the same)");
    {
        static const UINT ns[4][8] = {
            { 0x7fc00000, 0x3f000000, 0xffc00000, 0x7f800001, 0x3f000000, 0x3e800000, 0x3f000000, 0x3f000000 },
            { 0x7f800000, 0xff800000, 0x7fc00000, 0x3f000000, 0x7fc00000, 0xffc00000, 0x7f800000, 0x7fc00000 },
            { 0x3f000000, 0x7fc00000, 0x3e800000, 0x7fc00000, 0x3e800000, 0x3f000000, 0xff800000, 0x3f000000 },
            { 0xffc00000, 0x7f800000, 0xff800000, 0x3e800000, 0x7fc00000, 0x7fc00000, 0x7fc00000, 0x7fc00000 },
        };
        static const int bl9[9][3] = { { 2, 1, 1 }, { 2, 2, 1 }, { 5, 6, 1 }, { 9, 1, 1 }, { 16, 1, 1 }, { 1, 18, 1 }, { 2, 2, 2 }, { 2, 1, 4 }, { 2, 1, 5 } };
        for (int i = 0; i < 4; i++) {
            for (int k = 0; k < 8; k++) cbd[k] = ns[i][k];
            upload();
            printf("  %08x %08x %08x %08x / %08x %08x %08x %08x:", ns[i][0], ns[i][1], ns[i][2], ns[i][3], ns[i][4], ns[i][5], ns[i][6], ns[i][7]);
            for (int j = 0; j < 9; j++) {
                fill(&t16, &dsts[0]);
                ID3D11BlendState *st = blend2(TRUE, bl9[j][0], bl9[j][1], bl9[j][2], alpha_of(bl9[j][0]), alpha_of(bl9[j][1]), bl9[j][2], 0xf);
                ID3D11DeviceContext_OMSetBlendState(ctx, st, bf, 0xffffffff);
                ID3D11DeviceContext_Draw(ctx, 3, 0);
                show(&t16, 2, 1, 0);
                if (st) ID3D11BlendState_Release(st);
            }
            puts("");
        }
    }

    struct target t8;
    if (!target(&t8, DXGI_FORMAT_R8G8B8A8_SNORM, 4, W, H)) return 0;
    static const D3D11_RECT rects[2] = { { 1, 1, 3, 3 }, { 3, 0, 4, 2 } };
    static const float cvc[4] = { 0.5f, -0.5f, 0.25f, -1.0f };
    puts("ClearView of R8G8B8A8_SNORM (after ClearRenderTargetView 0.25 -0.75 1 -0.125) color 0.5 -0.5 0.25 -1 rects (1,1,3,3) (3,0,4,2): each texel");
    if (ctx1) {
        static const float c0[4] = { 0.25f, -0.75f, 1.0f, -0.125f };
        ID3D11DeviceContext_ClearRenderTargetView(ctx, t8.v, c0);
        ID3D11DeviceContext1_ClearView(ctx1, (ID3D11View *)t8.v, cvc, rects, 2);
        printf(" ");
        for (int i = 0; i < W * H; i++) show(&t8, i % W, i / W, 0);
        puts("");
    } else {
        puts("  no ID3D11DeviceContext1");
    }

    static const float dc[4] = { 64.0f / 127, -64.0f / 127, -1.0f, 48.0f / 127 };
    {
        UINT q = 0;
        ID3D11Device_CheckMultisampleQualityLevels(dev, DXGI_FORMAT_R16G16B16A16_SNORM, 4, &q);
        if (!q) {
            puts("no 4x R16G16B16A16_SNORM");
        } else {
            struct mstex m;
            if (!mstex(&m, DXGI_FORMAT_R16G16B16A16_SNORM, DXGI_FORMAT_R16G16B16A16_SNORM)) return 0;
            puts("4x multisampled R16G16B16A16_SNORM: cleared to 64/127 -64/127 -1 48/127, a half-covered triangle blended (src_alpha inv_src_alpha add, set 0), each pixel's 4 samples as float bits, then ResolveSubresource");
            ID3D11DeviceContext_ClearRenderTargetView(ctx, m.v, dc);
            half(m.v, srcs[0], 5, 6, 1);
            ms_rows(m.srv);
            printf("  resolved:");
            resolved(&t16, &m, DXGI_FORMAT_R16G16B16A16_SNORM);
            puts("  ResolveSubresource of samples each its own (ps_samples: each seed's pixels' resolved words)");
            ID3D11DeviceContext_PSSetShader(ctx, ps_samples, NULL, 0);
            ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &m.v, NULL);
            ID3D11DeviceContext_OMSetBlendState(ctx, NULL, zero, 0xffffffff);
            for (UINT seed = 0; seed < 65535; seed += 4369) {
                cbd[8] = seed;
                upload();
                ID3D11DeviceContext_Draw(ctx, 3, 0);
                printf("  seed16 %u:", seed);
                resolved(&t16, &m, DXGI_FORMAT_R16G16B16A16_SNORM);
            }
            if (ctx1) {
                puts("  ClearView (after ClearRenderTargetView 64/127 -64/127 -1 48/127) color 0.5 -0.5 0.25 -1 rects (1,1,3,3) (3,0,4,2): each pixel's 4 samples");
                ID3D11DeviceContext_ClearRenderTargetView(ctx, m.v, dc);
                ID3D11DeviceContext1_ClearView(ctx1, (ID3D11View *)m.v, cvc, rects, 2);
                ms_rows(m.srv);
            }
        }
    }
    {
        UINT q = 0;
        ID3D11Device_CheckMultisampleQualityLevels(dev, DXGI_FORMAT_R8G8B8A8_SNORM, 4, &q);
        if (!q) {
            puts("no 4x R8G8B8A8_SNORM");
        } else {
            struct mstex m;
            if (!mstex(&m, DXGI_FORMAT_R8G8B8A8_SNORM, DXGI_FORMAT_R8G8B8A8_SNORM)) return 0;
            if (ctx1) {
                puts("4x multisampled R8G8B8A8_SNORM: ClearView (after ClearRenderTargetView 64/127 -64/127 -1 48/127) color 0.5 -0.5 0.25 -1 rects (1,1,3,3) (3,0,4,2): each pixel's 4 samples");
                ID3D11DeviceContext_ClearRenderTargetView(ctx, m.v, dc);
                ID3D11DeviceContext1_ClearView(ctx1, (ID3D11View *)m.v, cvc, rects, 2);
                ms_rows(m.srv);
                printf("  resolved:");
                resolved(&t8, &m, DXGI_FORMAT_R8G8B8A8_SNORM);
            }
            struct mstex x, xu;
            if (!mstex(&x, DXGI_FORMAT_R8G8B8A8_TYPELESS, DXGI_FORMAT_R8G8B8A8_SNORM)) return 0;
            puts("4x multisampled R8G8B8A8_TYPELESS, R8G8B8A8_SNORM views: cleared to 64/127 -64/127 -1 48/127, a half-covered triangle blended (src_alpha inv_src_alpha add, set 0), each pixel's 4 samples as float bits, then ResolveSubresource as R8G8B8A8_SNORM");
            ID3D11DeviceContext_ClearRenderTargetView(ctx, x.v, dc);
            half(x.v, srcs[0], 5, 6, 1);
            ms_rows(x.srv);
            printf("  resolved:");
            resolved(&t8, &x, DXGI_FORMAT_R8G8B8A8_SNORM);
            puts("  ResolveSubresource of samples each its own (ps_samples, as SNORM: each seed's pixels' resolved words)");
            ID3D11DeviceContext_PSSetShader(ctx, ps_samples, NULL, 0);
            ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &x.v, NULL);
            ID3D11DeviceContext_OMSetBlendState(ctx, NULL, zero, 0xffffffff);
            for (UINT seed = 0; seed < 65535; seed += 13107) {
                cbd[8] = seed;
                upload();
                ID3D11DeviceContext_Draw(ctx, 3, 0);
                printf("  seed %u:", seed);
                resolved(&t8, &x, DXGI_FORMAT_R8G8B8A8_SNORM);
            }
            if (ctx1) {
                puts("  ClearView (after ClearRenderTargetView 64/127 -64/127 -1 48/127) color 0.5 -0.5 0.25 -1 rects (1,1,3,3) (3,0,4,2): each pixel's 4 samples");
                ID3D11DeviceContext_ClearRenderTargetView(ctx, x.v, dc);
                ID3D11DeviceContext1_ClearView(ctx1, (ID3D11View *)x.v, cvc, rects, 2);
                ms_rows(x.srv);
            }
            /* the same texture through R8G8B8A8_UNORM views: drawn (one one add of 0.25 0.5 0.75 1
             * over the SNORM half triangle's bytes) and read, then read as SNORM */
            puts("  through R8G8B8A8_UNORM views: cleared as SNORM to 64/127 -64/127 -1 48/127, the half-covered triangle (src_alpha inv_src_alpha add, set 0) as SNORM, then one one add of 0.25 0.5 0.75 1 as UNORM: the samples as UNORM, then as SNORM");
            ID3D11DeviceContext_ClearRenderTargetView(ctx, x.v, dc);
            half(x.v, srcs[0], 5, 6, 1);
            {
                D3D11_TEXTURE2D_DESC md;
                ID3D11Texture2D_GetDesc(x.t, &md);
                D3D11_RENDER_TARGET_VIEW_DESC rd;
                memset(&rd, 0, sizeof rd);
                rd.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
                rd.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE2DMS;
                D3D11_SHADER_RESOURCE_VIEW_DESC sd;
                memset(&sd, 0, sizeof sd);
                sd.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
                sd.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2DMS;
                xu.t = x.t;
                ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)x.t, &rd, &xu.v);
                ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)x.t, &sd, &xu.srv);
                static const float u4[4] = { 0.25f, 0.5f, 0.75f, 1.0f };
                for (int k = 0; k < 4; k++) cbd[k] = fbits(u4[k]);
                upload();
                ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
                ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &xu.v, NULL);
                ID3D11BlendState *st = blend2(TRUE, 2, 2, 1, 2, 2, 1, 0xf);
                ID3D11DeviceContext_OMSetBlendState(ctx, st, zero, 0xffffffff);
                ID3D11DeviceContext_Draw(ctx, 3, 0);
                if (st) ID3D11BlendState_Release(st);
                ms_rows(xu.srv);
                ms_rows(x.srv);
            }
        }
    }
    puts("done");
    return 0;
}
