/* Direct3D 11.1 logic operations (CreateBlendState1's LogicOpEnable / LogicOp) as the output
 * merger applies them:
 *  - each of the 16 D3D11_LOGIC_OPs on R8G8B8A8_UINT, R16G16_UINT, R32_UINT and
 *    R10G10B10A2_UINT targets: a pixel shader's constant source against a known destination;
 *  - a source wider than the channel (COPY, XOR, and without a logic op);
 *  - two triangles of one draw over the same pixels (XOR twice, OR then AND...: the second
 *    reads what the first wrote), a triangle strip's odd triangle, instances;
 *  - RenderTargetWriteMask with a logic op; a logic op on the second target only
 *    (IndependentBlendEnable), the first written plainly;
 *  - which formats CheckFeatureSupport(FORMAT_SUPPORT2) gives OUTPUT_MERGER_LOGIC_OP;
 *  - sources outside the channels of SINT and 16-bit UINT targets (no logic op, COPY,
 *    XOR, AND);
 *  - every op on targets of formats without it (UNORM, UNORM_SRGB, SNORM, SINT, FLOAT,
 *    B8G8R8A8, R11G11B10, R16_UNORM): what the output merger writes there;
 *  - two 4x multisampled R16G16B16A16_UINT targets (4 words, each in two channels), every
 *    sample read back through Texture2DMS: a full and a partly covered triangle, the pixel shader's SV_Position,
 *    an interpolated value and SV_Coverage under XOR over 0 (against no logic op: whether the
 *    shader still runs once a pixel), the blend sample mask, a sample-rate shader
 *    (SV_SampleIndex), and PSInvocations and a pixel shader's own count of its invocations.
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

static ID3D11Device *dev;
static ID3D11Device1 *dev1;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "cbuffer C : register(b0) { uint4 src; uint4 src1; float4 srcf; };\n"
    "float4 vs_full(uint id : SV_VertexID) : SV_Position { return float4((id >> 1) * 4.0 - 1.0, (id & 1) * 4.0 - 1.0, 0.5, 1); }\n"
    /* two full-target triangles per instance, the second with the first's corners */
    "float4 vs_twice(uint id : SV_VertexID) : SV_Position { uint k = id % 3; return float4((k >> 1) * 4.0 - 1.0, (k & 1) * 4.0 - 1.0, 0.5, 1); }\n"
    /* a strip of 4 over the whole target: triangles 0 1 2 and 1 2 3 (the odd one wound the other way) */
    "float4 vs_strip(uint id : SV_VertexID) : SV_Position { return float4((id >> 1) * 2.0 - 1.0, (id & 1) * 2.0 - 1.0, 0.5, 1); }\n"
    "uint4 ps(float4 p : SV_Position) : SV_Target { return src; }\n"
    "struct O2 { uint4 a : SV_Target0; uint4 b : SV_Target1; };\n"
    "O2 ps2(float4 p : SV_Position) { O2 o; o.a = src; o.b = src1; return o; }\n"
    "float4 psf(float4 p : SV_Position) : SV_Target { return srcf; }\n"
    "int4 psi(float4 p : SV_Position) : SV_Target { return asint(src); }\n"
    /* a triangle over half the target (clockwise): the pixels on its diagonal partly covered */
    "float4 vs_half(uint id : SV_VertexID) : SV_Position { return float4(id == 2 ? 1.0 : -1.0, id == 1 ? 1.0 : -1.0, 0.5, 1); }\n"
    "struct VI { float4 p : SV_Position; float t : T; };\n"
    "VI vs_attr(uint id : SV_VertexID) { VI o; o.p = float4(id == 2 ? 1.0 : -1.0, id == 1 ? 1.0 : -1.0, 0.5, 1); o.t = o.p.x * 0.37 + o.p.y * 0.11 + 0.5; return o; }\n"
    /* the 4x cases write 2 R16G16B16A16_UINT targets (WebGPU multisamples no 32-bit integer
     * format): 4 words, each in two channels */
    "struct O2w { uint4 a : SV_Target0; uint4 b : SV_Target1; };\n"
    "O2w o2w(uint4 v) { O2w o; o.a = uint4(v.x & 0xffff, v.x >> 16, v.y & 0xffff, v.y >> 16); o.b = uint4(v.z & 0xffff, v.z >> 16, v.w & 0xffff, v.w >> 16); return o; }\n"
    "O2w ps_const(float4 p : SV_Position) { return o2w(src); }\n"
    "O2w ps_attr(VI i, uint c : SV_Coverage) { return o2w(uint4(asuint(i.p.x), asuint(i.p.y), asuint(i.t), c)); }\n"
    "O2w ps_sample(float4 p : SV_Position, uint s : SV_SampleIndex) { return o2w(src ^ (s * 0x10001)); }\n"
    "RWByteAddressBuffer cnt : register(u2);\n"
    "O2w ps_count(float4 p : SV_Position) { uint o; cnt.InterlockedAdd(0, 1, o); return o2w(src); }\n"
    /* sample (x % 4) of pixel (x / 4, y) of the 2 multisampled targets, as 4 words */
    "Texture2DMS<uint4> m0 : register(t0); Texture2DMS<uint4> m1 : register(t1);\n"
    "uint4 ps_read(float4 p : SV_Position) : SV_Target { uint x = (uint)p.x; int2 q = int2(x / 4, (int)p.y); uint k = x % 4; uint4 a = m0.Load(q, k), b = m1.Load(q, k); return uint4(a.x | a.y << 16, a.z | a.w << 16, b.x | b.y << 16, b.z | b.w << 16); }\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "logic", NULL, NULL, entry, target, 0, 0, &b, &err);
    if (FAILED(hr)) printf("D3DCompile %s failed %#lx %s\n", entry, hr, err ? (char *)ID3D10Blob_GetBufferPointer(err) : "");
    if (err) ID3D10Blob_Release(err);
    return b;
}

static ID3D11Buffer *cb;

static void constants(const UINT s[4], const UINT s1[4], const float f[4]) {
    UINT d[12] = { 0 };
    memcpy(d, s, 16);
    if (s1) memcpy(d + 4, s1, 16);
    if (f) memcpy(d + 8, f, 16);
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)cb, 0, NULL, d, 0, 0);
}

struct target {
    ID3D11Texture2D *t, *staging;
    ID3D11RenderTargetView *v;
    DXGI_FORMAT f;
    UINT bpp;
};

static int target(struct target *t, DXGI_FORMAT f, UINT bpp) {
    D3D11_TEXTURE2D_DESC td = { W, H, 1, 1, f, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
    t->f = f;
    t->bpp = bpp;
    HRESULT hr = ID3D11Device_CreateTexture2D(dev, &td, NULL, &t->t);
    if (FAILED(hr)) { printf("CreateTexture2D %u failed %#lx\n", f, hr); return 0; }
    td.Usage = D3D11_USAGE_STAGING;
    td.BindFlags = 0;
    td.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    ID3D11Device_CreateTexture2D(dev, &td, NULL, &t->staging);
    ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)t->t, NULL, &t->v);
    return 1;
}

/* every texel of the target: `px` (bpp bytes, repeated) */
static void fill(struct target *t, const void *px) {
    unsigned char row[W * 8];
    for (int x = 0; x < W; x++) memcpy(row + x * t->bpp, px, t->bpp);
    unsigned char all[W * 8 * H];
    for (int y = 0; y < H; y++) memcpy(all + y * W * t->bpp, row, W * t->bpp);
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)t->t, 0, NULL, all, W * t->bpp, 0);
}

/* texel (x, y) as hex words */
static void show(struct target *t, int x, int y) {
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)t->staging, (ID3D11Resource *)t->t);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)t->staging, 0, D3D11_MAP_READ, 0, &m))) { printf(" map failed"); return; }
    const unsigned char *p = (const unsigned char *)m.pData + y * m.RowPitch + x * t->bpp;
    for (UINT k = 0; k < t->bpp; k += 4) {
        UINT v;
        memcpy(&v, p + k, 4);
        printf(" %08x", v);
    }
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)t->staging, 0);
}

/* the same texel on every pixel, or the first that differs */
static void uniform(struct target *t) {
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)t->staging, (ID3D11Resource *)t->t);
    D3D11_MAPPED_SUBRESOURCE m;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)t->staging, 0, D3D11_MAP_READ, 0, &m))) return;
    const unsigned char *p0 = m.pData;
    int bad = -1;
    for (int y = 0; y < H && bad < 0; y++)
        for (int x = 0; x < W; x++)
            if (memcmp((const unsigned char *)m.pData + y * m.RowPitch + x * t->bpp, p0, t->bpp)) { bad = y * W + x; break; }
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)t->staging, 0);
    if (bad >= 0) printf(" (pixel %d differs)", bad);
}

static ID3D11BlendState *blend(BOOL independent, UINT rt, BOOL logic, D3D11_LOGIC_OP op, UINT mask) {
    D3D11_BLEND_DESC1 b;
    memset(&b, 0, sizeof b);
    b.IndependentBlendEnable = independent;
    for (int i = 0; i < 8; i++) {
        b.RenderTarget[i].SrcBlend = b.RenderTarget[i].SrcBlendAlpha = D3D11_BLEND_ONE;
        b.RenderTarget[i].DestBlend = b.RenderTarget[i].DestBlendAlpha = D3D11_BLEND_ZERO;
        b.RenderTarget[i].BlendOp = b.RenderTarget[i].BlendOpAlpha = D3D11_BLEND_OP_ADD;
        b.RenderTarget[i].LogicOp = D3D11_LOGIC_OP_NOOP;
        b.RenderTarget[i].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
    }
    b.RenderTarget[rt].LogicOpEnable = logic;
    b.RenderTarget[rt].LogicOp = op;
    b.RenderTarget[rt].RenderTargetWriteMask = (UINT8)mask;
    ID3D11BlendState1 *s = NULL;
    HRESULT hr = ID3D11Device1_CreateBlendState1(dev1, &b, &s);
    if (FAILED(hr)) printf(" CreateBlendState1 failed %#lx", hr);
    return (ID3D11BlendState *)s;
}

static const char *OPS[16] = { "clear", "set", "copy", "copy_inverted", "noop", "invert", "and", "nand", "or", "nor", "xor", "equiv", "and_reverse", "and_inverted", "or_reverse", "or_inverted" };

int main(void) {
    D3D_FEATURE_LEVEL fl;
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx)) &&
        FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx))) {
        puts("skip no device");
        return 0;
    }
    if (fl < D3D_FEATURE_LEVEL_11_0) { puts("skip feature level below 11_0"); return 0; }
    if (FAILED(ID3D11Device_QueryInterface(dev, &IID_ID3D11Device1, (void **)&dev1))) { puts("skip no ID3D11Device1"); return 0; }
    D3D11_FEATURE_DATA_D3D11_OPTIONS o;
    memset(&o, 0, sizeof o);
    ID3D11Device_CheckFeatureSupport(dev, D3D11_FEATURE_D3D11_OPTIONS, &o, sizeof o);
    printf("options logic_op %d\n", o.OutputMergerLogicOp);
    if (!o.OutputMergerLogicOp) return 0;

    static const char *names[] = { "vs_full", "vs_twice", "vs_strip", "ps", "ps2", "psf", "psi" };
    static const char *kinds[] = { "vs_4_0", "vs_4_0", "vs_4_0", "ps_4_0", "ps_4_0", "ps_4_0", "ps_4_0" };
    ID3DBlob *bl[7];
    for (int i = 0; i < 7; i++)
        if (!(bl[i] = compile(names[i], kinds[i]))) return 0;
    ID3D11VertexShader *vs_full, *vs_twice, *vs_strip;
    ID3D11PixelShader *ps, *ps2, *psf, *psi;
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[0]), ID3D10Blob_GetBufferSize(bl[0]), NULL, &vs_full);
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[1]), ID3D10Blob_GetBufferSize(bl[1]), NULL, &vs_twice);
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(bl[2]), ID3D10Blob_GetBufferSize(bl[2]), NULL, &vs_strip);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[3]), ID3D10Blob_GetBufferSize(bl[3]), NULL, &ps);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[4]), ID3D10Blob_GetBufferSize(bl[4]), NULL, &ps2);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[5]), ID3D10Blob_GetBufferSize(bl[5]), NULL, &psf);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(bl[6]), ID3D10Blob_GetBufferSize(bl[6]), NULL, &psi);
    {
        D3D11_BUFFER_DESC bd = { 48, D3D11_USAGE_DEFAULT, D3D11_BIND_CONSTANT_BUFFER, 0, 0, 0 };
        ID3D11Device_CreateBuffer(dev, &bd, NULL, &cb);
    }
    ID3D11DeviceContext_PSSetConstantBuffers(ctx, 0, 1, &cb);
    D3D11_VIEWPORT vp = { 0, 0, W, H, 0, 1 };
    ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    float zero[4] = { 0, 0, 0, 0 };

    struct target t[4];
    static const DXGI_FORMAT fmts[4] = { DXGI_FORMAT_R8G8B8A8_UINT, DXGI_FORMAT_R16G16_UINT, DXGI_FORMAT_R32_UINT, DXGI_FORMAT_R10G10B10A2_UINT };
    static const char *fnames[4] = { "rgba8_uint", "rg16_uint", "r32_uint", "rgb10a2_uint" };
    /* each format's destination texel, and the source the shader gives (within the channels) */
    static const UINT dst[4] = { 0xc3a55a3c, 0xf00f0ff0, 0x12345678, 0x9abcdef0 };
    static const UINT srcs[4][4] = { { 0x96, 0x0f, 0xf0, 0x33 }, { 0x6996, 0x1ee1, 0, 0 }, { 0xdeadbeef, 0, 0, 0 }, { 0x2a5, 0x15a, 0x3c3, 1 } };
    for (int f = 0; f < 4; f++) {
        if (!target(&t[f], fmts[f], 4)) return 0;
        printf("%s: dst %08x src %x %x %x %x\n", fnames[f], dst[f], srcs[f][0], srcs[f][1], srcs[f][2], srcs[f][3]);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t[f].v, NULL);
        ID3D11DeviceContext_VSSetShader(ctx, vs_full, NULL, 0);
        ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
        constants(srcs[f], NULL, NULL);
        for (int op = 0; op < 16; op++) {
            fill(&t[f], &dst[f]);
            ID3D11BlendState *b = blend(FALSE, 0, TRUE, (D3D11_LOGIC_OP)op, 0xf);
            ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            printf("  %s:", OPS[op]);
            show(&t[f], 1, 2);
            uniform(&t[f]);
            puts("");
            if (b) ID3D11BlendState_Release(b);
        }
    }

    puts("a source wider than the channels (rgba8_uint, rgb10a2_uint)");
    {
        static const UINT wide[4] = { 0x12345601, 0x1ff, 0x100, 0xffffffff };
        constants(wide, NULL, NULL);
        for (int k = 0; k < 2; k++) {
            int f = k ? 3 : 0;
            ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t[f].v, NULL);
            for (int m = 0; m < 3; m++) {
                fill(&t[f], &dst[f]);
                ID3D11BlendState *b = blend(FALSE, 0, m > 0, m == 2 ? D3D11_LOGIC_OP_XOR : D3D11_LOGIC_OP_COPY, 0xf);
                ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
                ID3D11DeviceContext_Draw(ctx, 3, 0);
                printf("  %s %s:", fnames[f], m == 0 ? "no logic op" : m == 1 ? "copy" : "xor");
                show(&t[f], 0, 0);
                puts("");
                if (b) ID3D11BlendState_Release(b);
            }
        }
    }

    puts("two triangles of one draw over the same pixels (r32_uint)");
    {
        static const UINT s[4] = { 0x0ff000ff, 0, 0, 0 };
        constants(s, NULL, NULL);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t[2].v, NULL);
        ID3D11DeviceContext_VSSetShader(ctx, vs_twice, NULL, 0);
        static const D3D11_LOGIC_OP ops[] = { D3D11_LOGIC_OP_XOR, D3D11_LOGIC_OP_INVERT, D3D11_LOGIC_OP_EQUIV, D3D11_LOGIC_OP_NAND, D3D11_LOGIC_OP_AND_REVERSE, D3D11_LOGIC_OP_OR_INVERTED };
        static const char *on[] = { "xor", "invert", "equiv", "nand", "and_reverse", "or_inverted" };
        for (int i = 0; i < 6; i++) {
            for (int inst = 1; inst <= 2; inst++) {
                fill(&t[2], &dst[2]);
                ID3D11BlendState *b = blend(FALSE, 0, TRUE, ops[i], 0xf);
                ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
                ID3D11DeviceContext_DrawInstanced(ctx, 6, inst, 0, 0);
                printf("  %s, %d instance%s:", on[i], inst, inst > 1 ? "s" : "");
                show(&t[2], 2, 1);
                uniform(&t[2]);
                puts("");
                if (b) ID3D11BlendState_Release(b);
            }
        }
        /* a strip of two triangles: each pixel under one of them, once */
        ID3D11DeviceContext_VSSetShader(ctx, vs_strip, NULL, 0);
        ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        fill(&t[2], &dst[2]);
        ID3D11BlendState *b = blend(FALSE, 0, TRUE, D3D11_LOGIC_OP_XOR, 0xf);
        ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
        ID3D11DeviceContext_Draw(ctx, 4, 0);
        printf("  xor, a strip of 2:");
        for (int i = 0; i < W * H; i++) show(&t[2], i % W, i / W);
        puts("");
        if (b) ID3D11BlendState_Release(b);
        ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        ID3D11DeviceContext_VSSetShader(ctx, vs_full, NULL, 0);
    }

    puts("write masks with xor (rgba8_uint)");
    {
        constants(srcs[0], NULL, NULL);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &t[0].v, NULL);
        static const UINT masks[] = { 0, 1, 2, 4, 8, 5, 0xa, 0xe };
        for (int i = 0; i < 8; i++) {
            fill(&t[0], &dst[0]);
            ID3D11BlendState *b = blend(FALSE, 0, TRUE, D3D11_LOGIC_OP_XOR, masks[i]);
            ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            printf("  mask %x:", masks[i]);
            show(&t[0], 3, 3);
            puts("");
            if (b) ID3D11BlendState_Release(b);
        }
    }

    puts("a logic op on the second target only (rgba8_uint, r32_uint)");
    {
        constants(srcs[0], srcs[2], NULL);
        ID3D11RenderTargetView *two[2] = { t[0].v, t[2].v };
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 2, two, NULL);
        ID3D11DeviceContext_PSSetShader(ctx, ps2, NULL, 0);
        fill(&t[0], &dst[0]);
        fill(&t[2], &dst[2]);
        ID3D11BlendState *b = blend(TRUE, 1, TRUE, D3D11_LOGIC_OP_AND_INVERTED, 0xf);
        ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        printf("  rt0:");
        show(&t[0], 0, 0);
        printf(" rt1:");
        show(&t[2], 0, 0);
        puts("");
        if (b) ID3D11BlendState_Release(b);
        /* (without IndependentBlendEnable: target 0's state for both) */
        fill(&t[0], &dst[0]);
        fill(&t[2], &dst[2]);
        b = blend(FALSE, 0, TRUE, D3D11_LOGIC_OP_OR, 0xf);
        ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        printf("  both or: rt0:");
        show(&t[0], 0, 0);
        printf(" rt1:");
        show(&t[2], 0, 0);
        puts("");
        if (b) ID3D11BlendState_Release(b);
    }

    puts("a logic op on an R8G8B8A8_UNORM target");
    {
        struct target u;
        if (!target(&u, DXGI_FORMAT_R8G8B8A8_UNORM, 4)) return 0;
        static const float f[4] = { 0.25f, 0.5f, 1.0f, 0.0f };
        static const UINT none[4] = { 0, 0, 0, 0 };
        constants(none, NULL, f);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &u.v, NULL);
        ID3D11DeviceContext_PSSetShader(ctx, psf, NULL, 0);
        for (int op = 2; op < 16; op += 8) {
            fill(&u, &dst[0]);
            ID3D11BlendState *b = blend(FALSE, 0, TRUE, op == 2 ? D3D11_LOGIC_OP_COPY : D3D11_LOGIC_OP_XOR, 0xf);
            ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            printf("  %s:", op == 2 ? "copy" : "xor");
            show(&u, 0, 0);
            puts("");
            if (b) ID3D11BlendState_Release(b);
        }
    }
    puts("formats with OUTPUT_MERGER_LOGIC_OP (FORMAT_SUPPORT2 0x100)");
    {
        printf(" ");
        for (UINT f = 1; f <= 132; f++) {
            D3D11_FEATURE_DATA_FORMAT_SUPPORT2 s2 = { (DXGI_FORMAT)f, 0 };
            if (SUCCEEDED(ID3D11Device_CheckFeatureSupport(dev, D3D11_FEATURE_FORMAT_SUPPORT2, &s2, sizeof s2)) && (s2.OutFormatSupport2 & D3D11_FORMAT_SUPPORT2_OUTPUT_MERGER_LOGIC_OP)) printf(" %u", f);
        }
        puts("");
    }

    puts("every op on targets of formats without logic ops");
    {
        static const struct { DXGI_FORMAT f; const char *name; UINT bpp; int kind; UINT dst[2]; } nf[] = {
            { DXGI_FORMAT_R8G8B8A8_UNORM, "rgba8_unorm", 4, 0, { 0xc3a55a3c } },
            { DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, "rgba8_srgb", 4, 0, { 0xc3a55a3c } },
            { DXGI_FORMAT_B8G8R8A8_UNORM, "bgra8_unorm", 4, 0, { 0xc3a55a3c } },
            { DXGI_FORMAT_R8G8B8A8_SNORM, "rgba8_snorm", 4, 0, { 0xc3a55a3c } },
            { DXGI_FORMAT_R16_UNORM, "r16_unorm", 2, 0, { 0x5a3c } },
            { DXGI_FORMAT_R10G10B10A2_UNORM, "rgb10a2_unorm", 4, 0, { 0x9abcdef0 } },
            { DXGI_FORMAT_R11G11B10_FLOAT, "r11g11b10_float", 4, 0, { 0x3c003c00 } },
            { DXGI_FORMAT_R16G16_FLOAT, "rg16_float", 4, 0, { 0x3c00b800 } },
            { DXGI_FORMAT_R32_FLOAT, "r32_float", 4, 0, { 0x3f400000 } },
            { DXGI_FORMAT_R16G16B16A16_FLOAT, "rgba16_float", 8, 0, { 0x3c00b800, 0x40003400 } },
            { DXGI_FORMAT_R8G8B8A8_SINT, "rgba8_sint", 4, 1, { 0xc3a55a3c } },
            { DXGI_FORMAT_R32_SINT, "r32_sint", 4, 1, { 0x12345678 } },
        };
        static const float fs[4] = { 0.25f, 0.5f, 1.0f, 0.75f };
        static const UINT is[4] = { 0x16, 0xfffffff0, 0x70, 0x5 };
        for (int i = 0; i < (int)(sizeof nf / sizeof nf[0]); i++) {
            struct target u;
            if (!target(&u, nf[i].f, nf[i].bpp)) continue;
            printf("%s: dst", nf[i].name);
            for (UINT k = 0; k < nf[i].bpp; k += 4) printf(" %08x", nf[i].dst[k / 4]);
            if (nf[i].bpp == 2) printf(" %04x", nf[i].dst[0]);
            puts(nf[i].kind ? " src 16 fffffff0 70 5" : " src 0.25 0.5 1 0.75");
            constants(is, NULL, fs);
            ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &u.v, NULL);
            ID3D11DeviceContext_PSSetShader(ctx, nf[i].kind ? psi : psf, NULL, 0);
            for (int op = 0; op < 16; op++) {
                fill(&u, nf[i].dst);
                ID3D11BlendState *b = blend(FALSE, 0, TRUE, (D3D11_LOGIC_OP)op, 0xf);
                ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
                ID3D11DeviceContext_Draw(ctx, 3, 0);
                printf("  %s:", OPS[op]);
                if (u.bpp == 2) {
                    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)u.staging, (ID3D11Resource *)u.t);
                    D3D11_MAPPED_SUBRESOURCE m;
                    if (SUCCEEDED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)u.staging, 0, D3D11_MAP_READ, 0, &m))) {
                        printf(" %04x", *(const unsigned short *)((const unsigned char *)m.pData + 2 * m.RowPitch + 2));
                        ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)u.staging, 0);
                    }
                } else {
                    show(&u, 1, 2);
                    uniform(&u);
                }
                puts("");
                if (b) ID3D11BlendState_Release(b);
            }
            ID3D11Texture2D_Release(u.t);
            ID3D11Texture2D_Release(u.staging);
            ID3D11RenderTargetView_Release(u.v);
        }
        ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    }

    puts("sources outside the channels of SINT and 16-bit UINT targets");
    {
        static const struct { DXGI_FORMAT f; const char *name; UINT bpp; int kind; UINT dst[2]; } wf[] = {
            { DXGI_FORMAT_R8G8B8A8_SINT, "rgba8_sint", 4, 1, { 0xc3a55a3c } },
            { DXGI_FORMAT_R16G16_SINT, "rg16_sint", 4, 1, { 0xf00f0ff0 } },
            { DXGI_FORMAT_R16G16B16A16_SINT, "rgba16_sint", 8, 1, { 0xf00f0ff0, 0x12345678 } },
            { DXGI_FORMAT_R16G16B16A16_UINT, "rgba16_uint", 8, 0, { 0xf00f0ff0, 0x12345678 } },
        };
        static const UINT wide[4] = { 0x12345601, 0x1ff, 0xffff7f38, 0x80 };
        constants(wide, NULL, NULL);
        for (int i = 0; i < 4; i++) {
            struct target u;
            if (!target(&u, wf[i].f, wf[i].bpp)) continue;
            ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &u.v, NULL);
            ID3D11DeviceContext_PSSetShader(ctx, wf[i].kind ? psi : ps, NULL, 0);
            static const D3D11_LOGIC_OP wo[4] = { D3D11_LOGIC_OP_COPY, D3D11_LOGIC_OP_COPY, D3D11_LOGIC_OP_XOR, D3D11_LOGIC_OP_AND };
            static const char *wn[4] = { "no logic op", "copy", "xor", "and" };
            for (int m = 0; m < 4; m++) {
                fill(&u, wf[i].dst);
                ID3D11BlendState *b = blend(FALSE, 0, m > 0, wo[m], 0xf);
                ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, 0xffffffff);
                ID3D11DeviceContext_Draw(ctx, 3, 0);
                printf("  %s %s:", wf[i].name, wn[m]);
                show(&u, 0, 0);
                puts("");
                if (b) ID3D11BlendState_Release(b);
            }
            ID3D11Texture2D_Release(u.t);
            ID3D11Texture2D_Release(u.staging);
            ID3D11RenderTargetView_Release(u.v);
        }
        ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    }

    puts("two 4x multisampled R16G16B16A16_UINT targets (each pixel: its 4 samples, 4 words each: channels 0|1 2|3 of each target)");
    {
        D3D11_TEXTURE2D_DESC md = { W, H, 1, 1, DXGI_FORMAT_R16G16B16A16_UINT, { 4, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE, 0, 0 };
        ID3D11Texture2D *mt[2];
        ID3D11RenderTargetView *mv[2];
        ID3D11ShaderResourceView *ms[2];
        for (int i = 0; i < 2; i++) {
            if (FAILED(ID3D11Device_CreateTexture2D(dev, &md, NULL, &mt[i]))) { puts("  no 4x target"); puts("done"); return 0; }
            ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)mt[i], NULL, &mv[i]);
            ID3D11Device_CreateShaderResourceView(dev, (ID3D11Resource *)mt[i], NULL, &ms[i]);
        }
        /* the read-back: W * 4 x H, sample k of pixel x at x * 4 + k */
        struct target rd;
        {
            D3D11_TEXTURE2D_DESC td = { W * 4, H, 1, 1, DXGI_FORMAT_R32G32B32A32_UINT, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
            ID3D11Device_CreateTexture2D(dev, &td, NULL, &rd.t);
            td.Usage = D3D11_USAGE_STAGING;
            td.BindFlags = 0;
            td.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
            ID3D11Device_CreateTexture2D(dev, &td, NULL, &rd.staging);
            ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)rd.t, NULL, &rd.v);
            rd.bpp = 16;
        }
        ID3DBlob *mb[7];
        static const char *mn[7] = { "vs_half", "vs_attr", "ps_attr", "ps_sample", "ps_count", "ps_read", "ps_const" };
        static const char *mk[7] = { "vs_4_0", "vs_4_0", "ps_5_0", "ps_5_0", "ps_5_0", "ps_5_0", "ps_5_0" };
        for (int i = 0; i < 7; i++)
            if (!(mb[i] = compile(mn[i], mk[i]))) return 0;
        ID3D11VertexShader *vs_half, *vs_attr;
        ID3D11PixelShader *ps_attr, *ps_sample, *ps_count, *ps_read, *ps_const;
        ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(mb[0]), ID3D10Blob_GetBufferSize(mb[0]), NULL, &vs_half);
        ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(mb[1]), ID3D10Blob_GetBufferSize(mb[1]), NULL, &vs_attr);
        ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(mb[2]), ID3D10Blob_GetBufferSize(mb[2]), NULL, &ps_attr);
        ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(mb[3]), ID3D10Blob_GetBufferSize(mb[3]), NULL, &ps_sample);
        ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(mb[4]), ID3D10Blob_GetBufferSize(mb[4]), NULL, &ps_count);
        ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(mb[5]), ID3D10Blob_GetBufferSize(mb[5]), NULL, &ps_read);
        ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(mb[6]), ID3D10Blob_GetBufferSize(mb[6]), NULL, &ps_const);
        ID3D11Buffer *cbuf;
        ID3D11UnorderedAccessView *cuav;
        {
            D3D11_BUFFER_DESC bd = { 16, D3D11_USAGE_DEFAULT, D3D11_BIND_UNORDERED_ACCESS, 0, D3D11_RESOURCE_MISC_BUFFER_ALLOW_RAW_VIEWS, 0 };
            ID3D11Device_CreateBuffer(dev, &bd, NULL, &cbuf);
            D3D11_UNORDERED_ACCESS_VIEW_DESC ud;
            memset(&ud, 0, sizeof ud);
            ud.Format = DXGI_FORMAT_R32_TYPELESS;
            ud.ViewDimension = D3D11_UAV_DIMENSION_BUFFER;
            ud.Buffer.NumElements = 4;
            ud.Buffer.Flags = D3D11_BUFFER_UAV_FLAG_RAW;
            ID3D11Device_CreateUnorderedAccessView(dev, (ID3D11Resource *)cbuf, &ud, &cuav);
        }
        ID3D11Buffer *cstage;
        {
            D3D11_BUFFER_DESC bd = { 16, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ, 0, 0 };
            ID3D11Device_CreateBuffer(dev, &bd, NULL, &cstage);
        }
        D3D11_QUERY_DESC qd = { D3D11_QUERY_PIPELINE_STATISTICS, 0 };
        ID3D11Query *q;
        ID3D11Device_CreateQuery(dev, &qd, &q);
        static const float clear[2][4] = { { 90.0f, 15360.0f, 7456.0f, 7.0f }, { 1.0f, 65535.0f, 4660.0f, 32768.0f } };
        static const UINT ms_src[4] = { 0x0ff000ff, 0x00ff00ff, 0xf0f0f0f0, 0x80000001 };
        static const struct { const char *name; int vs; int ps; BOOL logic; D3D11_LOGIC_OP op; UINT mask; int all; } mc[] = {
            /* vs: 0 full, 1 half, 2 attr; ps: 0 ps, 1 ps_attr, 2 ps_sample, 3 ps_count */
            { "full, xor", 0, 0, TRUE, D3D11_LOGIC_OP_XOR, 0xffffffff, 0 },
            { "half, xor", 1, 0, TRUE, D3D11_LOGIC_OP_XOR, 0xffffffff, 1 },
            { "half, and_inverted", 1, 0, TRUE, D3D11_LOGIC_OP_AND_INVERTED, 0xffffffff, 1 },
            { "half, sample mask 5, xor", 1, 0, TRUE, D3D11_LOGIC_OP_XOR, 0x5, 1 },
            { "full, sample mask a, invert", 0, 0, TRUE, D3D11_LOGIC_OP_INVERT, 0xa, 0 },
            { "position, value, coverage, no logic op", 2, 1, FALSE, D3D11_LOGIC_OP_NOOP, 0xffffffff, 1 },
            { "position, value, coverage, xor over 0", 2, 1, TRUE, D3D11_LOGIC_OP_XOR, 0xffffffff, 1 },
            { "position, value, coverage, or over 0", 2, 1, TRUE, D3D11_LOGIC_OP_OR, 0xffffffff, 1 },
            { "position, value, coverage, sample mask 5, no logic op", 2, 1, FALSE, D3D11_LOGIC_OP_NOOP, 0x5, 1 },
            { "sample-rate shader, xor", 1, 2, TRUE, D3D11_LOGIC_OP_XOR, 0xffffffff, 1 },
            { "sample-rate shader, no logic op", 1, 2, FALSE, D3D11_LOGIC_OP_NOOP, 0xffffffff, 1 },
            { "counted, no logic op", 1, 3, FALSE, D3D11_LOGIC_OP_NOOP, 0xffffffff, 0 },
            { "counted, xor", 1, 3, TRUE, D3D11_LOGIC_OP_XOR, 0xffffffff, 0 },
            { "counted, copy", 1, 3, TRUE, D3D11_LOGIC_OP_COPY, 0xffffffff, 0 },
        };
        ID3D11VertexShader *vss[3] = { vs_full, vs_half, vs_attr };
        ID3D11PixelShader *pss[4] = { ps_const, ps_attr, ps_sample, ps_count };
        for (int c = 0; c < (int)(sizeof mc / sizeof mc[0]); c++) {
            BOOL over0 = strstr(mc[c].name, "over 0") != NULL;
            for (int i = 0; i < 2; i++) {
                static const float cz[4] = { 0, 0, 0, 0 };
                ID3D11DeviceContext_ClearRenderTargetView(ctx, mv[i], over0 ? cz : clear[i]);
            }
            static const UINT zc[4] = { 0, 0, 0, 0 };
            ID3D11DeviceContext_ClearUnorderedAccessViewUint(ctx, cuav, zc);
            constants(ms_src, NULL, NULL);
            D3D11_VIEWPORT v = { 0, 0, W, H, 0, 1 };
            ID3D11DeviceContext_RSSetViewports(ctx, 1, &v);
            ID3D11DeviceContext_OMSetRenderTargetsAndUnorderedAccessViews(ctx, 2, mv, NULL, 2, 1, &cuav, NULL);
            ID3D11DeviceContext_VSSetShader(ctx, vss[mc[c].vs], NULL, 0);
            ID3D11DeviceContext_PSSetShader(ctx, pss[mc[c].ps], NULL, 0);
            ID3D11BlendState *b = blend(FALSE, 0, mc[c].logic, mc[c].op, 0xf);
            ID3D11DeviceContext_OMSetBlendState(ctx, b, zero, mc[c].mask);
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)q);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)q);
            D3D11_QUERY_DATA_PIPELINE_STATISTICS st;
            memset(&st, 0, sizeof st);
            while (ID3D11DeviceContext_GetData(ctx, (ID3D11Asynchronous *)q, &st, sizeof st, 0) == S_FALSE) {}
            ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)cstage, (ID3D11Resource *)cbuf);
            UINT count = 0;
            D3D11_MAPPED_SUBRESOURCE m;
            if (SUCCEEDED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)cstage, 0, D3D11_MAP_READ, 0, &m))) {
                count = *(const UINT *)m.pData;
                ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)cstage, 0);
            }
            if (b) ID3D11BlendState_Release(b);
            /* read every sample back */
            ID3D11DeviceContext_OMSetBlendState(ctx, NULL, zero, 0xffffffff);
            ID3D11DeviceContext_OMSetRenderTargetsAndUnorderedAccessViews(ctx, 1, &rd.v, NULL, 1, 0, NULL, NULL);
            D3D11_VIEWPORT rv = { 0, 0, W * 4, H, 0, 1 };
            ID3D11DeviceContext_RSSetViewports(ctx, 1, &rv);
            ID3D11DeviceContext_VSSetShader(ctx, vs_full, NULL, 0);
            ID3D11DeviceContext_PSSetShader(ctx, ps_read, NULL, 0);
            ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 2, ms);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            ID3D11ShaderResourceView *none[2] = { NULL, NULL };
            ID3D11DeviceContext_PSSetShaderResources(ctx, 0, 2, none);
            printf("  %s: ps invocations %llu, counted %u\n", mc[c].name, (unsigned long long)st.PSInvocations, count);
            if (strncmp(mc[c].name, "counted", 7) == 0) continue;
            for (int y = 0; y < H; y++) {
                if (!mc[c].all && y != 1) continue;
                printf("   row %d:", y);
                for (int x = 0; x < W; x++) {
                    if (!mc[c].all && x != 1) continue;
                    printf(" |");
                    for (int k = 0; k < 4; k++) show(&rd, x * 4 + k, y);
                }
                puts("");
            }
        }
        ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
    }
    puts("done");
    return 0;
}
