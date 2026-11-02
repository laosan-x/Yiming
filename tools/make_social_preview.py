# -*- coding: utf-8 -*-
"""生成 GitHub 社交预览图 1280x640 (social-preview.png)"""
from PIL import Image, ImageDraw, ImageFilter, ImageFont
import os

W, H = 1280, 640
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# ---------- 背景：深色渐变 + 紫青氛围光 ----------
bg = Image.new("RGB", (W, H), "#0e1120")
px = bg.load()
top = (14, 17, 32)      # 0e1120
bot = (26, 32, 58)      # 1a203a
for y in range(H):
    t = y / H
    r = int(top[0] + (bot[0]-top[0]) * t)
    g = int(top[1] + (bot[1]-top[1]) * t)
    b = int(top[2] + (bot[2]-top[2]) * t)
    for x in range(0, W):
        px[x, y] = (r, g, b)

# 氛围光斑
glow = Image.new("RGB", (W, H), (0, 0, 0))
gd = ImageDraw.Draw(glow)
gd.ellipse([820, -220, 1420, 320], fill=(64, 48, 140))    # 右上紫
gd.ellipse([-180, 380, 320, 780], fill=(24, 84, 110))     # 左下青
glow = glow.filter(ImageFilter.GaussianBlur(120))
bg = Image.blend(bg, Image.composite(glow, Image.new("RGB", (W, H), (0,0,0)),
      Image.new("L", (W, H), 110)), 0.55)

draw = ImageDraw.Draw(bg)

# ---------- 字体 ----------
FD = "C:/Windows/Fonts"
f_title = ImageFont.truetype(f"{FD}/msyhbd.ttc", 58)
f_sub   = ImageFont.truetype(f"{FD}/msyh.ttc", 26)
f_pill  = ImageFont.truetype(f"{FD}/msyh.ttc", 20)
f_logo  = ImageFont.truetype(f"{FD}/msyhbd.ttc", 30)

# ---------- 标题区 ----------
tx, ty = 64, 66
draw.text((tx, ty), "壹鸣 Wonderland", font=f_title, fill=(255, 255, 255))
draw.text((tx + 4, ty + 92), "本地优先的智能抠图与证件照工作站", font=f_sub, fill=(158, 170, 205))

# ---------- 徽章 pills ----------
pills = ["Rust + Iced", "纯本地推理 · 无上传", "Apache-2.0"]
px_, py_ = 64, 208
for label in pills:
    tw = draw.textlength(label, font=f_pill)
    pw, ph = int(tw) + 36, 40
    draw.rounded_rectangle([px_, py_, px_ + pw, py_ + ph], radius=20,
                           fill=(38, 46, 78), outline=(86, 98, 140), width=1)
    draw.text((px_ + 18, py_ + 8), label, font=f_pill, fill=(196, 206, 235))
    px_ += pw + 14

# ---------- 三张应用截图：圆角 + 投影 + 底部一排 ----------
def rounded(im, rad):
    m = Image.new("L", im.size, 0)
    d = ImageDraw.Draw(m)
    d.rounded_rectangle([0, 0, im.size[0]-1, im.size[1]-1], radius=rad, fill=255)
    out = Image.new("RGBA", im.size)
    out.paste(im, (0, 0), m)
    return out

SCALE = 0.268                     # 1440x900 -> 386x241
sw, sh = int(1440*SCALE), int(900*SCALE)
GAP = 20
total = 3*sw + 2*GAP
sx0 = (W - total)//2
sy = 322

shots = []
for n in ["home", "matting", "idphoto"]:
    im = Image.open(os.path.join(ROOT, "docs", "screenshots", f"{n}.jpg")).convert("RGB")
    im = im.resize((sw, sh), Image.LANCZOS)
    shots.append(rounded(im, 10))

# 投影层
shadow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
sd = ImageDraw.Draw(shadow)
for i in range(3):
    x = sx0 + i*(sw+GAP)
    sd.rounded_rectangle([x+6, sy+10, x+sw+6, sy+sh+10], radius=12, fill=(0, 0, 0, 160))
shadow = shadow.filter(ImageFilter.GaussianBlur(10))
bg.paste(shadow, (0, 0), shadow)

for i, s in enumerate(shots):
    x = sx0 + i*(sw+GAP)
    bg.paste(s, (x, sy), s)
    # 细边框
    draw.rounded_rectangle([x, sy, x+sw-1, sy+sh-1], radius=10, outline=(90, 100, 140), width=1)

out = os.path.join(ROOT, "social-preview.png")
bg.save(out, "PNG")
print("saved:", out, bg.size, os.path.getsize(out)//1024, "KB")
