/* CopyResource and staging Map of a D24S8 attachment after a full or aspect-only clear.
 * Array slices have distinct clears; a depth-only draw leaves their stencil values intact.
 * libs: -ld3d11 -ld3dcompiler_47
 * run: browser */
#define COBJMACROS
#include "../probe.h"
#include <d3d11.h>
#include <d3dcompiler.h>

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

static void show(ID3D11Texture2D *stage, UINT layers) {
    for (UINT i = 0; i < layers; i++) {
        D3D11_MAPPED_SUBRESOURCE m;
        HRESULT hr = ID3D11DeviceContext_Map(ctx, (ID3D11Resource *)stage, i, D3D11_MAP_READ, 0, &m);
        if (FAILED(hr)) { printf("map %u failed %08lx\n", i, hr); continue; }
        const UINT *row = m.pData;
        printf("slice %u: %08x %08x %08x %08x\n", i, row[0], row[1], row[2], row[3]);
        ID3D11DeviceContext_Unmap(ctx, (ID3D11Resource *)stage, i);
    }
}

static void run(UINT layers) {
    ID3D11Texture2D *src, *dst, *stage;
    D3D11_TEXTURE2D_DESC td = { 4, 2, 1, layers, DXGI_FORMAT_D24_UNORM_S8_UINT, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_DEPTH_STENCIL, 0, 0 };
    if (FAILED(ID3D11Device_CreateTexture2D(dev, &td, NULL, &src)) ||
        FAILED(ID3D11Device_CreateTexture2D(dev, &td, NULL, &dst))) { puts("no depth texture"); return; }
    td.Usage = D3D11_USAGE_STAGING; td.BindFlags = 0; td.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    if (FAILED(ID3D11Device_CreateTexture2D(dev, &td, NULL, &stage))) { puts("no staging texture"); return; }

    ID3D11DepthStencilView *views[2];
    printf("layers %u\n", layers);
    for (UINT i = 0; i < layers; i++) {
        D3D11_DEPTH_STENCIL_VIEW_DESC dd;
        memset(&dd, 0, sizeof dd);
        dd.Format = td.Format;
        dd.ViewDimension = layers == 1 ? D3D11_DSV_DIMENSION_TEXTURE2D : D3D11_DSV_DIMENSION_TEXTURE2DARRAY;
        if (layers > 1) { dd.Texture2DArray.ArraySize = 1; dd.Texture2DArray.FirstArraySlice = i; }
        if (FAILED(ID3D11Device_CreateDepthStencilView(dev, (ID3D11Resource *)src, &dd, &views[i]))) { puts("no depth view"); return; }
        ID3D11DeviceContext_ClearDepthStencilView(ctx, views[i], D3D11_CLEAR_DEPTH | D3D11_CLEAR_STENCIL, i ? 0.75f : 0.25f, 0x51 + i);
    }
    D3D11_VIEWPORT vp = { 0, 0, 2, 2, 0, 1 };
    ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
    ID3D11DeviceContext_OMSetRenderTargets(ctx, 0, NULL, views[0]);
    ID3D11DeviceContext_Draw(ctx, 3, 0);
    ID3D11DeviceContext_OMSetRenderTargets(ctx, 0, NULL, NULL);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)dst, (ID3D11Resource *)src);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)stage, (ID3D11Resource *)dst);
    show(stage, layers);
    ID3D11DeviceContext_ClearDepthStencilView(ctx, views[layers - 1], D3D11_CLEAR_STENCIL, 0, 0xe1);
    ID3D11DeviceContext_CopyResource(ctx, (ID3D11Resource *)stage, (ID3D11Resource *)src);
    show(stage, layers);
    for (UINT i = 0; i < layers; i++) ID3D11DepthStencilView_Release(views[i]);
    ID3D11Texture2D_Release(stage);
    ID3D11Texture2D_Release(dst);
    ID3D11Texture2D_Release(src);
}

int main(void) {
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, NULL, &ctx))) {
        puts("no WARP device");
        return 1;
    }
    static const char src[] = "float4 vs(uint i : SV_VertexID) : SV_Position {"
                              " float2 p = float2((i << 1) & 2, i & 2);"
                              " return float4(p * float2(2,-2) + float2(-1,1), 0.5, 1); }";
    ID3DBlob *code;
    ID3D11VertexShader *vs;
    if (FAILED(D3DCompile(src, sizeof(src) - 1, "probe", NULL, NULL, "vs", "vs_5_0", 0, 0, &code, NULL)) ||
        FAILED(ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(code), ID3D10Blob_GetBufferSize(code), NULL, &vs))) {
        puts("no vertex shader");
        return 1;
    }
    ID3D10Blob_Release(code);
    ID3D11DeviceContext_VSSetShader(ctx, vs, NULL, 0);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    D3D11_DEPTH_STENCIL_DESC d = { 0 };
    d.DepthEnable = TRUE; d.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL; d.DepthFunc = D3D11_COMPARISON_ALWAYS;
    ID3D11DepthStencilState *ds;
    if (FAILED(ID3D11Device_CreateDepthStencilState(dev, &d, &ds))) { puts("no depth state"); return 1; }
    ID3D11DeviceContext_OMSetDepthStencilState(ctx, ds, 0);
    run(1);
    run(2);
    ID3D11DepthStencilState_Release(ds);
    ID3D11VertexShader_Release(vs);
    ID3D11DeviceContext_Release(ctx);
    ID3D11Device_Release(dev);
    return 0;
}
