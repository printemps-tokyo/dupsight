# dupsight

Find near-duplicate images in your folders: the same picture saved smaller,
recompressed, brightened, converted to another format or, with `--rotations`,
turned or mirrored. dupsight groups them, suggests the file to keep, and shows
each group side by side in an HTML page.

dupsight only reports. It never deletes, moves or changes a file.

```
$ dupsight scan ~/Pictures/trip
Group 1 (3 images; keep: the most pixels)
  keep  trip/IMG_2041.jpg          4032x3024    3.1 MB  jpeg
        trip/IMG_2041 (1).jpg      4032x3024    3.1 MB  jpeg  identical file
        trip/shared/IMG_2041.jpg   1600x1200    412 KB  jpeg  dHash  1  pHash  0

Group 2 (2 images; keep: same pixels, the largest file)
  keep  trip/beach.png             1920x1080    2.4 MB  png
        trip/beach.webp            1920x1080    188 KB  webp  dHash  0  pHash  2

2 groups, 5 images in them (3 besides the ones to keep); 214 images scanned; limits dHash 8, pHash 10.
```

## Install

```bash
cargo install --git https://github.com/printemps-tokyo/dupsight
```

Or download a binary for Linux (x86_64) or macOS (x86_64, arm64) from the
[releases](https://github.com/printemps-tokyo/dupsight/releases), with a
`.sha256` file next to each to check it. Building needs Rust 1.88 or later.

## Use

```bash
dupsight scan PHOTOS                      # groups, as text
dupsight scan PHOTOS --html report.html   # and a page to look at them side by side
dupsight scan PHOTOS --level strict       # only near-identical copies
dupsight scan PHOTOS --rotations          # also turned and mirrored copies
dupsight scan PHOTOS --json               # for scripts
dupsight scan PHOTOS --extras             # only the files besides the ones to keep
dupsight scan assets --check              # exit 1 if any group is found, for CI
dupsight compare a.jpg b.png              # how alike two images are
```

`scan` takes files and folders. Folders are walked, without following
symbolic links and without hidden files and folders unless `--hidden` is given.
Files that cannot be read are listed at the end and do not stop the scan.

Formats: JPEG, PNG, WebP, GIF (the first frame), BMP and TIFF. In folders,
files are picked by their extension (`.jpg`, `.jpeg`, `.png`, `.webp`, `.gif`,
`.bmp`, `.tif`, `.tiff`, in any case); each file is then read by its contents,
so a PNG named `.jpg` still works. Transparent areas are seen over white.

### Which file to keep

In each group dupsight suggests one file: the one with the most pixels; among
equal sizes, the largest file (the least compressed); among equal files, the
first by path. `keep:` on the group line says which of these decided.

Every other member of a group matches the kept file itself, within the
limits; members are never joined only through a third image. Each line shows
how far that file is from the kept one, or `identical file` when the bytes are
the same.

### Removing the extras

dupsight leaves this to you. Look at the HTML report first, then, for example:

```bash
dupsight scan PHOTOS --extras -0 | xargs -0 ls -l      # see them
dupsight scan PHOTOS --extras -0 | xargs -0 rm --      # remove them
```

`-0` ends each path with a NUL byte, so names with spaces or newlines are safe
with `xargs -0`.

## How alike is alike

Each image is reduced to a 64x64 grayscale thumbnail and given two 64-bit
perceptual hashes, as described by Neal Krawetz:

- dHash ([Kind of Like That](https://www.hackerfactor.com/blog/index.php?/archives/529-Kind-of-Like-That.html)):
  the thumbnail at 9x8, one bit for each pixel that is darker than its right
  neighbor.
- pHash ([Looks Like It](https://www.hackerfactor.com/blog/index.php?/archives/432-Looks-Like-It.html)):
  the lowest 8x8 frequencies of a 32x32 discrete cosine transform, one bit for
  each above their mean (the first, the average brightness, left out of the
  mean).

Two images are the same picture when both hashes differ in at most so many of
their 64 bits:

| `--level` | dHash | pHash | Finds |
| --- | --- | --- | --- |
| `strict` | 3 | 4 | the same picture resized, recompressed or converted |
| `normal` (default) | 8 | 10 | also brighter, darker, blurred or grayscale copies |
| `loose` | 12 | 14 | also small crops and heavier edits, with more chance of look-alikes |

`--max-dhash` and `--max-phash` set the limits directly. `dupsight compare`
shows both distances for two files, and the limits they were checked against.

For a sense of scale, these are the distances (dHash/pHash) we measured
between 13 photos, illustrations and banners and edited copies of each:

| Edit | dHash | pHash |
| --- | --- | --- |
| a quarter of the size | 0-3 | 0-1 |
| JPEG at quality 30 | 0-4 | 0-4 |
| grayscale | 0-2 | 0-4 |
| blurred | 0-2 | 0-2 |
| brighter | 0-5 | 0-8 |
| more contrast | 0-16 | 0-13 |
| 5% cropped from each side | 0-9 | 4-17 |
| 10% cropped from each side | 0-16 | 8-22 |

The closest two unrelated pictures among them were 12/31 and 16/25 apart.

### What it does not find

- Crops of more than a few percent, collages, and pictures inside other
  pictures. Perceptual hashes compare whole images.
- Turned or mirrored copies, unless `--rotations` is given. Then each image is
  hashed in all eight orientations, which is slower.
- Different photos of the same scene taken a moment apart are often found,
  since they do look almost the same; that is usually what you want, but check
  the report.

Screenshots of the same app or page look alike to these hashes even when the
details differ. Use `--level strict` for folders of screenshots.

## Output

`--json` prints one object:

```json
{
  "thresholds": {"dhash": 8, "phash": 10},
  "scanned": 214,
  "groups": [
    {
      "keepReason": "largest",
      "images": [
        {"path": "trip/IMG_2041.jpg", "width": 4032, "height": 3024, "bytes": 3251200, "format": "jpeg", "keep": true},
        {"path": "trip/shared/IMG_2041.jpg", "width": 1600, "height": 1200, "bytes": 421888, "format": "jpeg", "keep": false,
         "dhash": 1, "phash": 0, "orientation": 0, "identical": false}
      ]
    }
  ],
  "skipped": [{"path": "trip/broken.jpg", "reason": "..."}]
}
```

`keepReason` is `largest`, `largerFile` or `firstPath`. `orientation` is what
turns the file into the kept one: 0 as it is, 1-3 that many quarter turns
clockwise, 4-7 the same followed by a left-right mirror.

The HTML page loads the images from their files through `file://` links, so
open it on the same computer.

Exit status: 0 when done, 1 when `--check` found a group (or, for `compare`,
the pictures differ), 2 on an error such as a path that does not exist.

## Compared with other tools

[czkawka](https://github.com/qarmin/czkawka) finds similar images among many
other cleanup tasks and has a graphical interface. dupsight does one thing
from the command line, with output meant for scripts and CI, and never
touches your files.

## License

MIT
