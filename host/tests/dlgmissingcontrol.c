/* libs: -lcomctl32 -luser32 */
#include <windows.h>
#include <commctrl.h>
#include <stdio.h>
#include <string.h>

static unsigned inits;

static INT_PTR CALLBACK dialog_proc(HWND window, UINT msg, WPARAM wp, LPARAM lp)
{
    (void)window;
    (void)wp;
    (void)lp;
    if (msg == WM_INITDIALOG) ++inits;
    return FALSE;
}

static DLGTEMPLATE *template_with_missing_control(DWORD style)
{
    static _Alignas(4) BYTE bytes[256];
    memset(bytes, 0, sizeof(bytes));
    DLGTEMPLATE *dialog = (DLGTEMPLATE *)bytes;
    dialog->style = style;
    dialog->cdit = 1;
    dialog->cx = 80;
    dialog->cy = 55;
    /* 18-byte DLGTEMPLATE, then three empty WORD entries. */
    DLGITEMTEMPLATE *item = (DLGITEMTEMPLATE *)(bytes + 24);
    item->style = WS_CHILD | WS_VISIBLE;
    item->cx = 20;
    item->cy = 10;
    item->id = 101;
    lstrcpyW((WCHAR *)((BYTE *)item + 18), L"MasuMissingDialogControl");
    return dialog;
}

static unsigned initialized;
static int CALLBACK sheet_callback(HWND window, UINT message, LPARAM param)
{
    (void)window;
    (void)param;
    if (message == PSCB_INITIALIZED) ++initialized;
    return 0;
}

int main(void)
{
    INITCOMMONCONTROLSEX controls = {sizeof(controls), ICC_TAB_CLASSES};
    InitCommonControlsEx(&controls);
    HINSTANCE instance = GetModuleHandleW(NULL);
    for (unsigned no_fail = 0; no_fail <= 1; ++no_fail) {
        inits = 0;
        const DWORD style = WS_POPUP | (no_fail ? DS_NOFAILCREATE : 0);
        DLGTEMPLATE *templ = template_with_missing_control(style);
        SetLastError(0xdeadbeef);
        HWND dialog = CreateDialogIndirectParamW(instance, templ, NULL, dialog_proc, 0);
        DWORD error = GetLastError();
        printf("dialog nofail %u window %d initialized %u error %lu\n",
               no_fail, !!dialog, inits, dialog ? 0 : error);
        if (dialog) DestroyWindow(dialog);
    }

    for (unsigned modeless = 0; modeless <= 1; ++modeless) {
        inits = 0;
        PROPSHEETPAGEW page = {0};
        page.dwSize = sizeof(page);
        page.dwFlags = PSP_DLGINDIRECT;
        page.hInstance = instance;
        page.pResource = template_with_missing_control(WS_CHILD | DS_CONTROL);
        page.pfnDlgProc = dialog_proc;
        HPROPSHEETPAGE handle = CreatePropertySheetPageW(&page);
        if (!handle) return 1;

        PROPSHEETHEADERW sheet = {0};
        sheet.dwSize = sizeof(sheet);
        sheet.dwFlags = PSH_USECALLBACK | (modeless ? PSH_MODELESS : 0);
        sheet.pfnCallback = sheet_callback;
        initialized = 0;
        sheet.hInstance = instance;
        sheet.pszCaption = L"Missing control";
        sheet.nPages = 1;
        sheet.phpage = &handle;
        INT_PTR result = PropertySheetW(&sheet);
        HWND window = modeless && result > 0 ? (HWND)result : NULL;
        printf("sheet modeless %u result %ld window %d page initialized %u sheet initialized %u\n",
               modeless, modeless ? (long)(window != NULL) : (long)result,
               window ? IsWindow(window) : FALSE, inits, initialized);
        if (window) DestroyWindow(window);
    }
    return 0;
}
