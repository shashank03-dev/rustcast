from PIL import Image, ImageDraw, ImageFont

def rounded_rect_mask(size, radius, ss=4):
    # supersampled rounded-rect alpha mask
    big = size*ss
    m = Image.new("L", (big, big), 0)
    d = ImageDraw.Draw(m)
    d.rounded_rectangle([0,0,big-1,big-1], radius=radius*ss, fill=255)
    return m.resize((size,size), Image.LANCZOS)

def make_icon(size):
    ss = 4
    big = size*ss
    img = Image.new("RGBA", (big, big), (0,0,0,0))
    d = ImageDraw.Draw(img)
    # crimson tile with a subtle vertical depth (top a touch brighter)
    radius = int(big*0.235)  # Raycast-ish corner
    d.rounded_rectangle([0,0,big-1,big-1], radius=radius, fill=(214, 35, 45, 255))
    # glyph: bold white "R"
    try:
        font = ImageFont.truetype("/usr/share/fonts/truetype/ubuntu/Ubuntu-B.ttf", int(big*0.62))
    except Exception:
        font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", int(big*0.6))
    text = "R"
    bbox = d.textbbox((0,0), text, font=font)
    tw, th = bbox[2]-bbox[0], bbox[3]-bbox[1]
    tx = (big - tw)//2 - bbox[0]
    ty = (big - th)//2 - bbox[1]
    d.text((tx, ty), text, font=font, fill=(255,255,255,255))
    img = img.resize((size,size), Image.LANCZOS)
    return img

# main + multi-size
master = make_icon(512)
master.save("/home/user/rustcast/assets/icons/rustcast.png")
for s in (16,24,32,48,64,128,256):
    make_icon(s).save(f"/home/user/rustcast/assets/icons/rustcast-{s}.png")
# preview copy
master.save("/tmp/rustcast_icon_preview.png")
print("done")
