/* Direct3D 11 queries and predicates: CreateQuery/CreatePredicate of each D3D11_QUERY (and
 * MiscFlags), GetDataSize; GetData before End, while building, with wrong sizes and NULL data,
 * with DONOTFLUSH; an occlusion query, an occlusion predicate and PIPELINE_STATISTICS over
 * draws in viewports of known pixels (instanced too, depth-tested half away); SetPredication
 * with either value (whether the draw after it reached the target); an event; two timestamps
 * in a row (whether the second is later) inside TIMESTAMP_DISJOINT; a dispatch's
 * CSInvocations; DrawInstancedIndirect's vertices.
 * WARP or hardware on the Windows runner.
 * libs: -ld3d11 -ld3dcompiler_47
 * run: browser (sake's Direct3D 11 runs in the page: tests/winref/run-browser.mjs) */
#define COBJMACROS
#define INITGUID
#include "../probe.h"
#include <d3d11.h>
#include <d3dcompiler.h>

#define W 64
#define H 64

static ID3D11Device *dev;
static ID3D11DeviceContext *ctx;

static const char *SRC =
    "float4 vs(uint id : SV_VertexID) : SV_Position {\n"
    "    return float4((id & 1) * 4.0 - 1.0, (id >> 1) * 4.0 - 1.0, 0.5, 1);\n"
    "}\n"
    "float4 vs_near(uint id : SV_VertexID) : SV_Position {\n"
    "    return float4((id & 1) * 4.0 - 1.0, (id >> 1) * 4.0 - 1.0, 0.25, 1);\n"
    "}\n"
    "float4 ps(float4 p : SV_Position) : SV_Target { return float4(1, 0, 1, 1); }\n"
    "RWTexture2D<float4> u : register(u0);\n"
    "[numthreads(4, 2, 3)] void cs(uint3 i : SV_DispatchThreadID) { u[i.xy] = float4(1, 1, 1, 1); }\n";

static ID3DBlob *compile(const char *entry, const char *target) {
    ID3DBlob *b = NULL, *err = NULL;
    HRESULT hr = D3DCompile(SRC, strlen(SRC), "query", NULL, NULL, entry, target, 0, 0, &b, &err);
    if (err) ID3D10Blob_Release(err);
    if (FAILED(hr)) printf("D3DCompile %s failed %#lx\n", entry, hr);
    return b;
}

static void viewport(float x, float y, float w, float h) {
    D3D11_VIEWPORT vp = { x, y, w, h, 0, 1 };
    ID3D11DeviceContext_RSSetViewports(ctx, 1, &vp);
}

static HRESULT wait(ID3D11Asynchronous *q, void *data, UINT size) {
    HRESULT hr;
    int n = 0;
    while ((hr = ID3D11DeviceContext_GetData(ctx, q, data, size, 0)) == S_FALSE && n < 1000000) n++;
    return hr;
}

static ID3D11Query *query(D3D11_QUERY t) {
    D3D11_QUERY_DESC d = { t, 0 };
    ID3D11Query *q = NULL;
    ID3D11Device_CreateQuery(dev, &d, &q);
    return q;
}

int main(void) {
    D3D_FEATURE_LEVEL fl;
    if (FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx)) &&
        FAILED(D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_WARP, NULL, 0, NULL, 0, D3D11_SDK_VERSION, &dev, &fl, &ctx))) {
        puts("skip no device");
        return 0;
    }
    puts("CreateQuery / CreatePredicate of types 0..16, MiscFlags 0 and PREDICATEHINT");
    for (int t = 0; t <= 16; t++) {
        for (UINT misc = 0; misc <= 1; misc++) {
            D3D11_QUERY_DESC d = { (D3D11_QUERY)t, misc };
            ID3D11Query *q = NULL;
            ID3D11Predicate *p = NULL;
            HRESULT a = ID3D11Device_CreateQuery(dev, &d, &q);
            HRESULT b = ID3D11Device_CreatePredicate(dev, &d, &p);
            HRESULT c = ID3D11Device_CreateQuery(dev, &d, NULL);
            printf("  %d/%u: query %#lx predicate %#lx NULL %#lx size %u\n", t, misc, a, b, c, q ? ID3D11Asynchronous_GetDataSize((ID3D11Asynchronous *)q) : 0);
            if (q) ID3D11Query_Release(q);
            if (p) ID3D11Predicate_Release(p);
        }
    }

    D3D11_TEXTURE2D_DESC td = { W, H, 1, 1, DXGI_FORMAT_R8G8B8A8_UNORM, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET | D3D11_BIND_UNORDERED_ACCESS, 0, 0 };
    ID3D11Texture2D *rt = NULL, *ds = NULL;
    ID3D11Device_CreateTexture2D(dev, &td, NULL, &rt);
    D3D11_TEXTURE2D_DESC dd = { W, H, 1, 1, DXGI_FORMAT_D24_UNORM_S8_UINT, { 1, 0 }, D3D11_USAGE_DEFAULT, D3D11_BIND_DEPTH_STENCIL, 0, 0 };
    ID3D11Device_CreateTexture2D(dev, &dd, NULL, &ds);
    ID3D11RenderTargetView *rtv = NULL;
    ID3D11DepthStencilView *dsv = NULL;
    ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)rt, NULL, &rtv);
    ID3D11Device_CreateDepthStencilView(dev, (ID3D11Resource *)ds, NULL, &dsv);
    ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &rtv, NULL);
    ID3DBlob *vsb = compile("vs", "vs_4_0"), *nearb = compile("vs_near", "vs_4_0"), *psb = compile("ps", "ps_4_0"), *csb = compile("cs", "cs_5_0");
    if (!vsb || !nearb || !psb || !csb) return 0;
    ID3D11VertexShader *vs = NULL, *vsn = NULL;
    ID3D11PixelShader *ps = NULL;
    ID3D11ComputeShader *cs = NULL;
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(vsb), ID3D10Blob_GetBufferSize(vsb), NULL, &vs);
    ID3D11Device_CreateVertexShader(dev, ID3D10Blob_GetBufferPointer(nearb), ID3D10Blob_GetBufferSize(nearb), NULL, &vsn);
    ID3D11Device_CreatePixelShader(dev, ID3D10Blob_GetBufferPointer(psb), ID3D10Blob_GetBufferSize(psb), NULL, &ps);
    ID3D11Device_CreateComputeShader(dev, ID3D10Blob_GetBufferPointer(csb), ID3D10Blob_GetBufferSize(csb), NULL, &cs);
    ID3D11DeviceContext_VSSetShader(ctx, vs, NULL, 0);
    ID3D11DeviceContext_PSSetShader(ctx, ps, NULL, 0);
    ID3D11DeviceContext_IASetPrimitiveTopology(ctx, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);

    ID3D11Query *occ = query(D3D11_QUERY_OCCLUSION), *st = query(D3D11_QUERY_PIPELINE_STATISTICS);
    ID3D11Predicate *pred = NULL;
    {
        D3D11_QUERY_DESC d = { D3D11_QUERY_OCCLUSION_PREDICATE, 0 };
        ID3D11Device_CreatePredicate(dev, &d, &pred);
    }
    if (!occ || !st || !pred) { puts("skip no occlusion queries"); return 0; }
    puts("GetData of an occlusion query");
    {
        UINT64 v = 7;
        printf("  never ended: %#lx\n", ID3D11DeviceContext_GetData(ctx, (ID3D11Asynchronous *)occ, &v, 8, 0));
        ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)occ);
        printf("  building: %#lx\n", ID3D11DeviceContext_GetData(ctx, (ID3D11Asynchronous *)occ, &v, 8, 0));
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)occ);
        HRESULT hr = wait((ID3D11Asynchronous *)occ, NULL, 0);
        printf("  ended, nothing drawn: %#lx", hr);
        printf(" size 4 %#lx size 16 %#lx NULL/8 %#lx", ID3D11DeviceContext_GetData(ctx, (ID3D11Asynchronous *)occ, &v, 4, 0),
            ID3D11DeviceContext_GetData(ctx, (ID3D11Asynchronous *)occ, &v, 16, 0), ID3D11DeviceContext_GetData(ctx, (ID3D11Asynchronous *)occ, NULL, 8, 0));
        hr = ID3D11DeviceContext_GetData(ctx, (ID3D11Asynchronous *)occ, &v, 8, 0);
        printf(" 8 %#lx = %llu\n", hr, (unsigned long long)v);
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)occ);
        hr = wait((ID3D11Asynchronous *)occ, &v, 8);
        printf("  End without Begin: %#lx %llu\n", hr, (unsigned long long)v);
    }

    puts("occlusion / predicate / pipeline statistics over draws");
    static const struct { float x, y, w, h; UINT inst; int inside; } dr[] = {
        { 0, 0, 10, 10, 1, 1 }, { 20, 20, 3, 7, 2, 1 }, { 0, 0, 64, 64, 1, 0 }, { 60, 60, 10, 10, 1, 1 },
    };
    for (int pass = 0; pass < 2; pass++) {
        ID3D11Asynchronous *qs[3] = { (ID3D11Asynchronous *)occ, (ID3D11Asynchronous *)pred, (ID3D11Asynchronous *)st };
        for (int k = 0; k < 3; k++) ID3D11DeviceContext_Begin(ctx, qs[k]);
        for (int i = 0; i < 4; i++) {
            if (pass == 1 && dr[i].inside) continue;
            if (!dr[i].inside) for (int k = 0; k < 3; k++) ID3D11DeviceContext_End(ctx, qs[k]);
            viewport(dr[i].x, dr[i].y, dr[i].w, dr[i].h);
            ID3D11DeviceContext_DrawInstanced(ctx, 3, dr[i].inst, 0, 0);
            if (!dr[i].inside) {
                UINT64 v = 0;
                wait(qs[0], &v, 8);
                for (int k = 0; k < 3; k++) ID3D11DeviceContext_Begin(ctx, qs[k]);
            }
        }
        for (int k = 0; k < 3; k++) ID3D11DeviceContext_End(ctx, qs[k]);
        UINT64 v = 0;
        BOOL any = 7;
        D3D11_QUERY_DATA_PIPELINE_STATISTICS s;
        HRESULT a = wait(qs[0], &v, 8), b = wait(qs[1], &any, 4), c = wait(qs[2], &s, sizeof s);
        printf("  %s: occlusion %#lx %llu predicate %#lx %d\n", pass ? "nothing inside" : "10x10 + 2 x 3x7 + 4x4", a, (unsigned long long)v, b, any);
        printf("  statistics %#lx: IA %llu %llu VS %llu GS %llu %llu C %llu %llu PS %llu HS %llu DS %llu CS %llu\n", c, (unsigned long long)s.IAVertices, (unsigned long long)s.IAPrimitives,
            (unsigned long long)s.VSInvocations, (unsigned long long)s.GSInvocations, (unsigned long long)s.GSPrimitives, (unsigned long long)s.CInvocations, (unsigned long long)s.CPrimitives,
            (unsigned long long)s.PSInvocations, (unsigned long long)s.HSInvocations, (unsigned long long)s.DSInvocations, (unsigned long long)s.CSInvocations);
        for (BOOL value = 0; value <= 1; value++) {
            float clear[4] = { 0, 0, 0, 0 };
            ID3D11DeviceContext_ClearRenderTargetView(ctx, rtv, clear);
            ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)occ);
            ID3D11DeviceContext_SetPredication(ctx, pred, value);
            viewport(0, 0, 8, 8);
            ID3D11DeviceContext_Draw(ctx, 3, 0);
            ID3D11DeviceContext_SetPredication(ctx, NULL, FALSE);
            ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)occ);
            wait((ID3D11Asynchronous *)occ, &v, 8);
            printf("  SetPredication(predicate, %d): the draw after it %llu samples\n", value, (unsigned long long)v);
        }
    }
    {
        UINT64 v = 0;
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &rtv, dsv);
        ID3D11DeviceContext_ClearDepthStencilView(ctx, dsv, D3D11_CLEAR_DEPTH, 1.0f, 0);
        ID3D11DeviceContext_VSSetShader(ctx, vsn, NULL, 0);
        viewport(0, 0, 16, 32);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        ID3D11DeviceContext_VSSetShader(ctx, vs, NULL, 0);
        ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)occ);
        viewport(0, 0, 32, 32);
        ID3D11DeviceContext_Draw(ctx, 3, 0);
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)occ);
        HRESULT hr = wait((ID3D11Asynchronous *)occ, &v, 8);
        printf("  32x32 behind a 16x32 nearer half: %#lx %llu\n", hr, (unsigned long long)v);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &rtv, NULL);
    }

    puts("event, timestamps, dispatch, indirect");
    {
        ID3D11Query *ev = query(D3D11_QUERY_EVENT), *ts = query(D3D11_QUERY_TIMESTAMP), *dj = query(D3D11_QUERY_TIMESTAMP_DISJOINT);
        BOOL done = 7;
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)ev);
        HRESULT h = ID3D11DeviceContext_GetData(ctx, (ID3D11Asynchronous *)ev, &done, 4, D3D11_ASYNC_GETDATA_DONOTFLUSH);
        printf("  event DONOTFLUSH first: %s", h == S_FALSE ? "S_FALSE" : h == S_OK ? "S_OK" : "other");
        h = wait((ID3D11Asynchronous *)ev, &done, 4);
        printf("; then %#lx %d\n", h, done);
        UINT64 t0 = 0, t1 = 0;
        D3D11_QUERY_DATA_TIMESTAMP_DISJOINT d = { 0, 7 };
        ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)dj);
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)ts);
        HRESULT a = wait((ID3D11Asynchronous *)ts, &t0, 8);
        viewport(0, 0, W, H);
        for (int i = 0; i < 50; i++) ID3D11DeviceContext_Draw(ctx, 3, 0);
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)ts);
        HRESULT b = wait((ID3D11Asynchronous *)ts, &t1, 8);
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)dj);
        HRESULT c = wait((ID3D11Asynchronous *)dj, &d, sizeof d);
        printf("  timestamps %#lx %#lx later %d; disjoint %#lx frequency nonzero %d disjoint %d\n", a, b, t1 > t0, c, d.Frequency != 0, d.Disjoint);

        ID3D11UnorderedAccessView *uav = NULL;
        ID3D11Device_CreateUnorderedAccessView(dev, (ID3D11Resource *)rt, NULL, &uav);
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 0, NULL, NULL);
        ID3D11DeviceContext_CSSetShader(ctx, cs, NULL, 0);
        ID3D11DeviceContext_CSSetUnorderedAccessViews(ctx, 0, 1, &uav, NULL);
        D3D11_QUERY_DATA_PIPELINE_STATISTICS s;
        ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)st);
        ID3D11DeviceContext_Dispatch(ctx, 2, 3, 1);
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)st);
        h = wait((ID3D11Asynchronous *)st, &s, sizeof s);
        printf("  Dispatch(2, 3, 1) of 4x2x3: %#lx CS %llu IA %llu\n", h, (unsigned long long)s.CSInvocations, (unsigned long long)s.IAVertices);

        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &rtv, NULL);
        UINT args[4] = { 6, 2, 0, 0 };
        D3D11_BUFFER_DESC bd = { sizeof args, D3D11_USAGE_DEFAULT, 0, 0, D3D11_RESOURCE_MISC_DRAWINDIRECT_ARGS, 0 };
        D3D11_SUBRESOURCE_DATA init = { args, 0, 0 };
        ID3D11Buffer *ab = NULL;
        ID3D11Device_CreateBuffer(dev, &bd, &init, &ab);
        ID3D11DeviceContext_Begin(ctx, (ID3D11Asynchronous *)st);
        ID3D11DeviceContext_DrawInstancedIndirect(ctx, ab, 0);
        ID3D11DeviceContext_End(ctx, (ID3D11Asynchronous *)st);
        h = wait((ID3D11Asynchronous *)st, &s, sizeof s);
        printf("  DrawInstancedIndirect {6, 2, 0, 0}: %#lx IA %llu %llu\n", h, (unsigned long long)s.IAVertices, (unsigned long long)s.IAPrimitives);
    }
    return 0;
}
