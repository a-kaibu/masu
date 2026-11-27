#include <windows.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
static int failed;
#define CHECK(label,expr) do{int ok=!!(expr);printf("%s: %s\n",label,ok?"ok":"FAIL");failed|=!ok;}while(0)
int main(void) {
    HMODULE crt=LoadLibraryA("ucrtbase.dll");
    void *(__cdecl *resize)(void *,size_t,size_t)=(void *)GetProcAddress(crt,"_recalloc");
    int *(__cdecl *error)(void)=(void *)GetProcAddress(crt,"_errno");
    CHECK("recalloc export",resize&&error);if(!resize||!error)return 1;
    unsigned char *p=resize(NULL,8,1);int zero=p!=NULL;
    if(p)for(int i=0;i<8;i++)zero&=p[i]==0;
    CHECK("initial zero",zero);if(!p)return 1;
    memset(p,0x53,8);p=resize(p,8,4);
    int preserve=p!=NULL;if(p)for(int i=0;i<8;i++)preserve&=p[i]==0x53;
    CHECK("preserve old bytes",preserve);if(!p)return 1;
    zero=1;for(int i=8;i<32;i++)zero&=p[i]==0;
    CHECK("zero extension",zero);
    void *bad=resize(p,SIZE_MAX,2);
    CHECK("overflow preserves allocation",bad==NULL&&*error()==12&&p[0]==0x53);
    p=resize(p,4,1);CHECK("shrink",p&&p[0]==0x53&&p[3]==0x53);
    CHECK("zero size frees",resize(p,0,1)==NULL);

    unsigned char *(__cdecl *aligned)(void *,size_t,size_t,size_t)=(void *)GetProcAddress(crt,"_aligned_recalloc");
    intptr_t (__cdecl *heap)(void)=(void *)GetProcAddress(crt,"_get_heap_handle");
    CHECK("heap handle",heap&&(HANDLE)heap()==GetProcessHeap());
    CHECK("aligned recalloc export",aligned);if(!aligned)return 1;

    p=aligned(NULL,7,1,64);
    zero=p!=NULL;
    if(p)for(int i=0;i<7;i++)zero&=p[i]==0;
    CHECK("aligned initial zero",zero&&(uintptr_t)p%64==0);if(!p)return 1;
    memset(p,0x53,7);
    p=aligned(p,13,1,64);
    preserve=p!=NULL;
    if(p)for(int i=0;i<7;i++)preserve&=p[i]==0x53;
    CHECK("aligned growth preserves old bytes",preserve&&(uintptr_t)p%64==0);if(!p)return 1;
    zero=1;for(int i=7;i<13;i++)zero&=p[i]==0;
    CHECK("aligned zero extension",zero);
    p=aligned(p,13,1,128);
    CHECK("aligned realignment",p&&(uintptr_t)p%128==0&&p[6]==0x53);if(!p)return 1;
    bad=aligned(p,SIZE_MAX,2,128);
    CHECK("aligned overflow preserves allocation",bad==NULL&&*error()==12&&p[6]==0x53);
    p=aligned(p,4,1,128);
    CHECK("aligned shrink",p&&p[0]==0x53&&p[3]==0x53);
    CHECK("aligned zero size frees",aligned(p,0,1,128)==NULL);
    return failed;
}
