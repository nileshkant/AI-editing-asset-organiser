# Image-analysis dependency notices

Exact source versions are pinned in Cargo.lock. The adjacent files retain the
upstream license/copyright text for image 0.25.10, image-webp 0.2.4, PNG 0.18.1,
zune-jpeg 0.5.15, zune-core 0.5.3, moxcms 0.8.1 and 0.9.1, pxfm 0.1.30,
quick-error 2.0.1 and byteorder-lite 0.1.0.

MIT is selected where offered; BSD-3-Clause is selected for moxcms and pxfm.
All are permissively licensed. These texts were copied from the exact downloaded
crate archives, not reconstructed. Existing common dependencies retain their
own license obligations. Ship these notices alongside the image runtime when
its worker is packaged/enabled; SS-043 owns that integration.

Pillow 11.3.0/libwebp 1.5.0 generated the three synthetic test assets only.
Their runtime/code is not included in the application or this repository.
