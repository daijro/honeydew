#!/usr/bin/env python3
"""Regenerate BanaForge icons from the user-supplied banana image (requires Pillow)."""
from pathlib import Path
from io import BytesIO
import base64
from PIL import Image
ROOT = Path(__file__).resolve().parents[2]
BRAND = ROOT / 'assets/branding'
source = Image.open(BRAND / 'banana-source.webp').convert('RGBA')
# Source already has a clean alpha channel. Keep the complete fruit and its proportions.
fruit = source.crop(source.getbbox())

def icon(size, fill=0.88, mono=False, opaque=False):
    canvas = Image.new('RGBA', (size, size), '#17191c' if opaque else (0, 0, 0, 0))
    scaled = fruit.copy()
    scaled.thumbnail((round(size * fill), round(size * fill)), Image.Resampling.LANCZOS)
    if mono:
        mask = scaled.getchannel('A')
        scaled = Image.new('RGBA', scaled.size, 'white')
        scaled.putalpha(mask)
    canvas.alpha_composite(scaled, ((size-scaled.width)//2, (size-scaled.height)//2))
    return canvas

def svg(im):
    out=BytesIO(); im.save(out,format='PNG')
    return '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><image width="512" height="512" href="data:image/png;base64,'+base64.b64encode(out.getvalue()).decode()+'"/></svg>\n'

for size in (16,32,256): icon(size).save(BRAND / f'banaforge-{size}.png')
for size in (48,64,128,256): icon(size,mono=True).save(BRAND / f'banaforge-mono-{size}.png')
icon(512,fill=.76,opaque=True).save(BRAND/'banaforge-ios-512.png')
(BRAND/'banaforge.svg').write_text(svg(icon(512)))
(BRAND/'banaforge-mono.svg').write_text(svg(icon(512,mono=True)))
for size in (16,32,48,64,128,256,512):
    icon(size).save(ROOT / f'assets/linux/icons/hicolor/{size}x{size}/apps/banaforge.png')
(ROOT/'assets/linux/icons/hicolor/scalable/apps/banaforge.svg').write_text(svg(icon(512)))
res=ROOT/'android/app/src/main/res'
icon(512).save(res/'drawable-nodpi/banaforge.png')
# Raster layers are padded to the 66/108 adaptive safe zone. Bitmap XML layers
# work on our minSdk 26; this preserves the supplied image, not a vector approximation.
for name,fill,mono in [('banaforge_launcher',.60,False),('banaforge_themed',.60,True),('banaforge_splash',.60,False)]:
    icon(432,fill=fill,mono=mono).save(res/f'drawable-nodpi/{name}.png')
for name,bitmap in [('ic_launcher_foreground','banaforge_launcher'),('ic_launcher_monochrome','banaforge_themed'),('ic_splash_logo','banaforge_splash')]:
    (res/f'drawable/{name}.xml').write_text('<?xml version="1.0" encoding="utf-8"?>\n<bitmap xmlns:android="http://schemas.android.com/apk/res/android" android:src="@drawable/'+bitmap+'" android:gravity="fill" android:filter="true" />\n')
print('Regenerated desktop, web, UI, iOS, Android adaptive, themed and splash icons.')
