/* FillMode WIREFRAME as the rasterizer gives it: which triangles a wireframe draw culls (each
 * CullMode with FrontCounterClockwise off and on, a triangle of each winding), the depth its
 * edges write with DepthBias, SlopeScaledDepthBias and DepthBiasClamp (32-bit float, 24-bit and
 * 16-bit UNORM depth), the same for solid triangles and for a line list, what SV_Position.z the
 * pixel shader is given, and the edges of a triangle the near plane cuts. Each pixel the draw
 * reaches: the pixel shader's SV_Position (asuint), how many times it ran there, the depth
 * buffer's word after the draw. sake drew wireframe triangles as lines: no culling, and the
 * depth bias dropped (review b5dacf4 3.2).
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * lines: all
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11.h>
#include <d3dcompiler.h>

#define W 16
#define H 8
/* records: x y z w of SV_Position, the times the pixel shader ran there */
#define REC 5
#define WORDS (W * H * REC)
#define EMPTY 0xdeadbeefu

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "cbuffer V : register(b0) { float4 P[8]; };\n"
    "float4 vs(uint id : SV_VertexID) : SV_Position { return P[id]; }\n"
    "RWByteAddressBuffer u : register(u1);\n"
    "float4 ps(float4 p : SV_Position) : SV_Target {\n"
    "    uint b = ((uint)p.y * 16 + (uint)p.x) * 20; uint n;\n"
    "    u.Store4(b, asuint(p)); u.InterlockedAdd(b + 16, 1, n); return float4(1, 1, 1, 1); }\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "wire", NULL, NULL, entry, target, 0, 0, &b, &err);
    if (FAILED(hr)) printf("D3DCompile %s failed %#lx %s\n", entry, hr, err ? (char *)ID3D10Blob_GetBufferPointer(err) : "");
    if (err) ID3D10Blob_Release(err);
    return b;
}

static ID3D11Buffer *cb, *ub, *stage;
static ID3D11UnorderedAccessView *uav;
static ID3D11RenderTargetView *rt;
static ID3D11DepthStencilState *always;

enum { D32, D24, D16, NDS };
static const DXGI_FORMAT DSF[NDS] = { DXGI_FORMAT_D32_FLOAT, DXGI_FORMAT_D24_UNORM_S8_UINT, DXGI_FORMAT_D16_UNORM };
static const char *DSN[NDS] = { "D32_FLOAT", "D24_UNORM_S8_UINT", "D16_UNORM" };
static ID3D11Texture2D *dst[NDS], *dstage[NDS];
static ID3D11DepthStencilView *dsv[NDS];

static void vertices(const float (*p)[4], int n) {
    float d[32];
    memset(d, 0, sizeof d);
    for (int i = 0; i < n && i < 8; i++) memcpy(d + i * 4, p[i], 16);
    ID3D11DeviceContext_UpdateSubresource(ctx, (ID3D11Resource *)cb, 0, NULL, d, 0, 0);
}

static const char *CULL[] = { "", "none", "front", "back" };

/* one draw of `n` vertices with the rasterizer state given, every pixel it reached printed */
static void run(const char *name, D3D11_FILL_MODE fill, D3D11_CULL_MODE cull, BOOL ccw, INT bias, FLOAT clamp, FLOAT slope, int ds, D3D11_PRIMITIVE_TOPOLOGY topo, int n) {
    printf("%s, %s cull %s%s, bias %d clamp %g slope %g, %s:\n", name, fill == D3D11_FILL_WIREFRAME ? "wireframe" : "solid", CULL[cull], ccw ? " ccw" : "", bias, clamp, slope, DSN[ds]);
    D3D11_RASTERIZER_DESC rd = { fill, cull, ccw, bias, clamp, slope, TRUE, FALSE, FALSE, FALSE };
    ID3D11RasterizerState *rs;
    if (FAILED(ID3D11Device_CreateRasterizerState(dev, &rd, &rs))) { puts("  no rasterizer state"); return; }
    static const UINT e[4] = { EMPTY, EMPTY, EMPTY, EMPTY };
    ID3D11DeviceContext_ClearUnorderedAccessViewUint(ctx, uav, e);
    ID3D11DeviceContext_ClearDepthStencilView(ctx, dsv[ds], D3D11_CLEAR_DEPTH | D3D11_CLEAR_STENCIL, 1.0f, 0);
    ID3D11DeviceContext_OMSetRenderTargetsAndUnorderedAccessViews(ctx, 1, &rt, dsv[ds], 1, 1, &uav, NULL);
    ID3D11DeviceContext_OMSetDepthStencilState(ctx, always, 0);
    D3D11_VIEWPORT full = { 0, 0, W, H, 0, 1 };
    ID3D11DeviceContext_RSSetViewports(ctx, 1, &full);
    ID3D11DeviceContext_RSSetState(ctx, rs);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, topo);
    ID3D11DeviceContext_Draw(ctx, n, 0);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)stage, (ID3D11Resource *)ub);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)dstage[ds], (ID3D11Resource *)dst[ds]);
    D3D11_MAPPED_SUBRESOURCE m, dm;
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)stage, 0, D3D11_MAP_READ, 0, &m))) { puts("  map failed"); return; }
    if (FAILED(ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)dstage[ds], 0, D3D11_MAP_READ, 0, &dm))) { puts("  depth map failed"); return; }
    const UINT *w = m.pData;
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++) {
            const UINT *r = w + (y * W + x) * REC;
            const BYTE *row = (const BYTE *)dm.pData + y * dm.RowPitch;
            UINT dw = ds == D16 ? ((const USHORT *)row)[x] : ((const UINT *)row)[x];
            UINT clear = ds == D32 ? 0x3f800000u : ds == D24 ? 0x00ffffffu : 0xffffu;
            if (r[4] == EMPTY && dw == clear) continue;
            printf("  %d,%d: %08x %08x %08x %08x x%u depth %08x\n", x, y, r[0], r[1], r[2], r[3], r[4] == EMPTY ? 0 : r[4] - EMPTY, dw);
        }
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)dstage[ds], 0);
    ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)stage, 0);
    ID3D11RasterizerState_Release(rs);
}

int main(void) {
    D3D_FEATURE_LEVEL fl;
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx)) &&
        FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx))) {
        puts("skip no device");
        return 0;
    }
    if (fl < D3D_FEATURE_LEVEL_11_0) { puts("skip feature level below 11_0"); return 0; }
    ID3D11VertexShader *vs;
    ID3D11PixelShader *ps;
    ID3DBlob *b = compile("vs", "vs_5_0");
    if (!b) return 0;
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(b), ID3D10Blob_GetBufferSize(b), NULL, &vs);
    ID3D10Blob_Release(b);
    if (!(b = compile("ps", "ps_5_0"))) return 0;
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(b), ID3D10Blob_GetBufferSize(b), NULL, &ps);
    ID3D10Blob_Release(b);
    ID3D11DeviceContext_VSSetShader(ctx, vs, NULL, 0);
    ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    {
        D3D11_BUFFER_DESC bd = { 128, D3D11_USAGE_DEFAULT, D3D11_BIND_CONSTANT_BUFFER, 0, 0, 0 };
        ID3D11Device_CreateBuffer(dev, &bd, NULL, &cb);
        ID3D11DeviceContext_VSSetConstantBuffers(ctx, 0, 1, &cb);
    }
    {
        D3D11_BUFFER_DESC bd = { WORDS * 4, D3D11_USAGE_DEFAULT, D3D11_BIND_UNORDERED_ACCESS, 0, D3D11_RESOURCE_MISC_BUFFER_ALLOW_RAW_VIEWS, 0 };
        ID3D11Device_CreateBuffer(dev, &bd, NULL, &ub);
        D3D11_UNORDERED_ACCESS_VIEW_DESC ud;
        memset(&ud, 0, sizeof ud);
        ud.Format = DXGI_FORMAT_R32_TYPELESS;
        ud.ViewDimension = D3D11_UAV_DIMENSION_BUFFER;
        ud.Buffer.NumElements = WORDS;
        ud.Buffer.Flags = D3D11_BUFFER_UAV_FLAG_RAW;
        ID3D11Device_CreateUnorderedAccessView(dev, (ID3D11Resource *)ub, &ud, &uav);
        D3D11_BUFFER_DESC sd = { WORDS * 4, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ, 0, 0 };
        ID3D11Device_CreateBuffer(dev, &sd, NULL, &stage);
    }
    {
        D3D11_TEXTURE2D_DESC td = { W, H, 1, 1, DXGI_FORMAT_R8G8B8A8_UNORM, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, 0, 0 };
        ID3D11Texture2D *t;
        if (FAILED(ID3D11Device_CreateTexture2D(dev, &td, NULL, &t))) { puts("skip no target"); return 0; }
        ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)t, NULL, &rt);
    }
    for (int k = 0; k < NDS; k++) {
        D3D11_TEXTURE2D_DESC td = { W, H, 1, 1, DSF[k], { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_DEPTH_STENCIL, 0, 0 };
        if (FAILED(ID3D11Device_CreateTexture2D(dev, &td, NULL, &dst[k]))) { printf("skip no %s\n", DSN[k]); return 0; }
        ID3D11Device_CreateDepthStencilView(dev, (ID3D11Resource *)dst[k], NULL, &dsv[k]);
        td.Usage = D3D11_USAGE_STAGING;
        td.BindFlags = 0;
        td.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
        ID3D11Device_CreateTexture2D(dev, &td, NULL, &dstage[k]);
    }
    {
        D3D11_DEPTH_STENCIL_DESC dd;
        memset(&dd, 0, sizeof dd);
        dd.DepthEnable = TRUE;
        dd.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
        dd.DepthFunc = D3D11_COMPARISON_ALWAYS;
        ID3D11Device_CreateDepthStencilState(dev, &dd, &always);
    }

    /* two triangles: the left one clockwise on the target (Direct3D's front by default), the
     * right one counterclockwise */
    static const float two[6][4] = {
        { -0.9f, -0.7f, 0.5f, 1 }, { -0.5f, 0.8f, 0.5f, 1 }, { -0.1f, -0.7f, 0.5f, 1 },
        { 0.1f, -0.7f, 0.5f, 1 }, { 0.5f, 0.8f, 0.5f, 1 }, { 0.9f, -0.7f, 0.5f, 1 },
    };
    float tw[6][4];
    memcpy(tw, two, sizeof tw);
    /* (the right one's last two corners swapped: counterclockwise) */
    memcpy(tw[4], two[5], 16);
    memcpy(tw[5], two[4], 16);
    vertices(tw, 6);
    for (int c = D3D11_CULL_NONE; c <= D3D11_CULL_BACK; c++)
        for (int ccw = 0; ccw < 2; ccw++) {
            run("cw left, ccw right", D3D11_FILL_WIREFRAME, c, ccw, 0, 0, 0, D32, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 6);
            run("cw left, ccw right", D3D11_FILL_SOLID, c, ccw, 0, 0, 0, D32, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 6);
        }
    /* the same as a strip: its second triangle's winding the other way */
    {
        static const float sp[4][4] = { { -0.9f, -0.7f, 0.5f, 1 }, { -0.6f, 0.8f, 0.5f, 1 }, { 0.2f, -0.7f, 0.5f, 1 }, { 0.6f, 0.8f, 0.5f, 1 } };
        vertices(sp, 4);
        for (int c = D3D11_CULL_FRONT; c <= D3D11_CULL_BACK; c++) run("strip", D3D11_FILL_WIREFRAME, c, FALSE, 0, 0, 0, D32, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, 4);
    }

    /* depth bias: a triangle whose depth rises across the target */
    {
        static const float zp[3][4] = { { -0.9f, -0.8f, 0.2f, 1 }, { -0.2f, 0.9f, 0.4f, 1 }, { 0.9f, -0.6f, 0.7f, 1 } };
        vertices(zp, 3);
        static const struct { INT bias; FLOAT clamp, slope; } bs[] = {
            { 0, 0, 0 }, { 1000, 0, 0 }, { -1000, 0, 0 }, { 0, 0, 0.5f }, { 0, 0, -2.0f }, { 1000, 0, 0.5f }, { 1000, 0.0001f, 0.5f }, { -1000, -0.00005f, 0 }, { 100000, 0, 0 },
        };
        for (int ds = 0; ds < NDS; ds++)
            for (unsigned k = 0; k < sizeof bs / sizeof bs[0]; k++) {
                run("depth", D3D11_FILL_WIREFRAME, D3D11_CULL_NONE, FALSE, bs[k].bias, bs[k].clamp, bs[k].slope, ds, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3);
                run("depth", D3D11_FILL_SOLID, D3D11_CULL_NONE, FALSE, bs[k].bias, bs[k].clamp, bs[k].slope, ds, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3);
            }
        /* the bias past the far plane: clamped to the viewport's depth, or cut */
        run("depth", D3D11_FILL_WIREFRAME, D3D11_CULL_NONE, FALSE, 0, 0, 1000.0f, D32, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3);
        run("depth", D3D11_FILL_SOLID, D3D11_CULL_NONE, FALSE, 0, 0, 1000.0f, D32, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3);
    }

    /* a line list: the bias of a line */
    {
        static const float lp[4][4] = { { -0.95f, -0.6f, 0.2f, 1 }, { 0.9f, 0.7f, 0.6f, 1 }, { -0.3f, 0.95f, 0.5f, 1 }, { 0.1f, -0.9f, 0.5f, 1 } };
        vertices(lp, 4);
        run("lines", D3D11_FILL_SOLID, D3D11_CULL_BACK, FALSE, 1000, 0, 0.5f, D32, D3D11_PRIMITIVE_TOPOLOGY_LINELIST, 4);
        run("lines", D3D11_FILL_SOLID, D3D11_CULL_BACK, FALSE, 1000, 0, 0.5f, D24, D3D11_PRIMITIVE_TOPOLOGY_LINELIST, 4);
    }

    /* a triangle the near plane cuts, in wireframe */
    {
        static const float np[3][4] = { { -0.8f, -0.8f, -0.5f, 1 }, { 0.0f, 1.35f, 0.75f, 1.5f }, { 0.9f, -0.6f, 0.9f, 1 } };
        vertices(np, 3);
        run("clipped near", D3D11_FILL_WIREFRAME, D3D11_CULL_NONE, FALSE, 0, 0, 0, D32, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3);
        run("clipped near", D3D11_FILL_WIREFRAME, D3D11_CULL_NONE, FALSE, 1000, 0, 0.5f, D32, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3);
    }
    /* a triangle the far plane cuts */
    {
        static const float fp[3][4] = { { -0.8f, -0.8f, 0.3f, 1 }, { 0.0f, 0.9f, 1.6f, 1 }, { 0.9f, -0.6f, 0.6f, 1 } };
        vertices(fp, 3);
        run("clipped far", D3D11_FILL_WIREFRAME, D3D11_CULL_NONE, FALSE, 0, 0, 0, D32, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 3);
    }

    /* triangles of no area: three corners on a line, all three at one point */
    {
        static const float za[6][4] = {
            { -0.9f, -0.6f, 0.5f, 1 }, { -0.4f, 0.1f, 0.5f, 1 }, { 0.1f, 0.8f, 0.5f, 1 },
            { 0.5f, 0.25f, 0.5f, 1 }, { 0.5f, 0.25f, 0.5f, 1 }, { 0.5f, 0.25f, 0.5f, 1 },
        };
        vertices(za, 6);
        run("no area", D3D11_FILL_WIREFRAME, D3D11_CULL_NONE, FALSE, 0, 0, 0, D32, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 6);
        run("no area", D3D11_FILL_WIREFRAME, D3D11_CULL_BACK, FALSE, 0, 0, 0, D32, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, 6);
    }
    puts("done");
    return 0;
}
