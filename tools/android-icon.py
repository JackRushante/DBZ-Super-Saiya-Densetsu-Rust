"""Writes the Android launcher icon from its pixel art (assets/icon/sfera-4-stelle-54.png, 54 x 54, one pixel = 2 dp of
the adaptive icon's 108 dp) as vector drawables: each layer is one path per colour, a rectangle per run of pixels, so the
icon stays sharp at every density and the repository holds no picture outside assets/.

    python tools/android-icon.py

Layers: the ball (the pixels joined to the centre that are not the background's blues) is the foreground; the background
is the rest, with the ball's place filled by the blue around it; the monochrome layer (themed icons, Android 13+) is the
ball's outline with its stars cut out.
"""
import os
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ART = os.path.join(ROOT, 'assets', 'icon', 'sfera-4-stelle-54.png')
RES = os.path.join(ROOT, 'android', 'app', 'src', 'main', 'res')
BLUES = {0x101038, 0x182060, 0x203088, 0x2840A8, 0x7080D0}
STARS = {0xE82020, 0x981018}
FILL = 0x2840A8


def runs(pixels, size):
    """`{colour: path data}`: a rectangle per horizontal run of one colour."""
    paths = {}
    for y in range(size):
        x = 0
        while x < size:
            colour = pixels[y][x]
            end = x
            while end + 1 < size and pixels[y][end + 1] == colour:
                end += 1
            if colour is not None:
                paths.setdefault(colour, []).append(f'M{x},{y}h{end - x + 1}v1h-{end - x + 1}z')
            x = end + 1
    return paths


def vector(paths, size):
    body = ''.join(
        f'    <path android:fillColor="#{colour:06X}" android:pathData="{"".join(data)}" />\n'
        for colour, data in sorted(paths.items()))
    return ('<?xml version="1.0" encoding="utf-8"?>\n'
            '<!-- Written by tools/android-icon.py from assets/icon/sfera-4-stelle-54.png: edit the picture, not this. -->\n'
            '<vector xmlns:android="http://schemas.android.com/apk/res/android"\n'
            f'    android:width="108dp" android:height="108dp"\n'
            f'    android:viewportWidth="{size}" android:viewportHeight="{size}">\n{body}</vector>\n')


def main():
    image = Image.open(ART).convert('RGB')
    size = image.width
    assert image.size == (size, size) == (54, 54), image.size
    colour = [[(lambda p: p[0] << 16 | p[1] << 8 | p[2])(image.getpixel((x, y))) for x in range(size)]
              for y in range(size)]
    # the ball: what is joined to the centre without crossing the background's blues
    ball, todo = set(), [(size // 2, size // 2)]
    while todo:
        x, y = todo.pop()
        if (x, y) in ball or not (0 <= x < size and 0 <= y < size) or colour[y][x] in BLUES:
            continue
        ball.add((x, y))
        todo += [(x + dx, y + dy) for dx in (-1, 0, 1) for dy in (-1, 0, 1) if dx or dy]
    layers = {
        'ic_launcher_foreground': [[colour[y][x] if (x, y) in ball else None for x in range(size)] for y in range(size)],
        'ic_launcher_background': [[FILL if (x, y) in ball else colour[y][x] for x in range(size)] for y in range(size)],
        'ic_launcher_monochrome': [[0xFFFFFF if (x, y) in ball and colour[y][x] not in STARS else None
                                    for x in range(size)] for y in range(size)],
    }
    os.makedirs(os.path.join(RES, 'drawable'), exist_ok=True)
    for name, pixels in layers.items():
        with open(os.path.join(RES, 'drawable', name + '.xml'), 'w', newline='\n') as f:
            f.write(vector(runs(pixels, size), size))
    adaptive = ('<?xml version="1.0" encoding="utf-8"?>\n'
                '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
                '    <background android:drawable="@drawable/ic_launcher_background" />\n'
                '    <foreground android:drawable="@drawable/ic_launcher_foreground" />\n'
                '    <monochrome android:drawable="@drawable/ic_launcher_monochrome" />\n'
                '</adaptive-icon>\n')
    os.makedirs(os.path.join(RES, 'mipmap-anydpi-v26'), exist_ok=True)
    for name in ('ic_launcher', 'ic_launcher_round'):
        with open(os.path.join(RES, 'mipmap-anydpi-v26', name + '.xml'), 'w', newline='\n') as f:
            f.write(adaptive)
    print(f'ball {len(ball)} pixels; layers written to {RES}')


if __name__ == '__main__':
    main()
