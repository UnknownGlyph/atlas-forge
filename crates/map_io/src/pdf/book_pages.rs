use super::{text, Face, Faces, Page, Setting, PT};
use crate::dossier::{asset_id, Block, Paragraph, Publication, Span};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

const W: f64 = 794.0;
const H: f64 = 1123.0;
const M: f64 = 54.0;
type ImageRef = (usize, u32, u32);

struct Pdf {
    objects: Vec<Vec<u8>>,
}
impl Pdf {
    fn add(&mut self, obj: impl Into<Vec<u8>>) -> usize {
        self.objects.push(obj.into());
        self.objects.len()
    }
    fn stream(raw: &[u8], extra: &str) -> Vec<u8> {
        let z = miniz_oxide::deflate::compress_to_vec_zlib(raw, 6);
        let mut stream = format!(
            "<< /Length {} /Filter /FlateDecode {extra} >>\nstream\n",
            z.len()
        )
        .into_bytes();
        stream.extend_from_slice(&z);
        stream.extend_from_slice(b"\nendstream");
        stream
    }
    fn finish(self) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(object);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
        );
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}
struct Sheet {
    page: Page,
    used: BTreeSet<u64>,
    y: f64,
}
impl Sheet {
    fn new() -> Self {
        Self {
            page: Page {
                ops: format!("{PT} 0 0 -{PT} 0 {:.2} cm\n", H * PT),
                alphas: BTreeMap::new(),
            },
            used: BTreeSet::new(),
            y: M,
        }
    }
}
fn layout_text(spans: &[Span], size: f64, width: f64) -> Vec<Vec<Span>> {
    let mut rows = vec![Vec::<Span>::new()];
    let mut used = 0.0;
    for span in spans {
        let replace;
        let source = if span.image {
            replace = format!("[图片：{}]", span.text);
            &replace
        } else {
            &span.text
        };
        let mut buffer = String::new();
        for c in source.chars() {
            let mut utf8 = [0; 4];
            let wf = f64::from(crate::scene::text_width(
                c.encode_utf8(&mut utf8),
                size as f32,
            ));
            if (used + wf > width && used > 0.0) || c == '\n' {
                if !buffer.is_empty() {
                    rows.last_mut().unwrap().push(Span {
                        text: std::mem::take(&mut buffer),
                        ..span.clone()
                    });
                }
                rows.push(Vec::new());
                used = 0.0;
            }
            if c != '\n' {
                buffer.push(c);
                used += wf;
            }
        }
        if !buffer.is_empty() {
            rows.last_mut().unwrap().push(Span {
                text: buffer,
                ..span.clone()
            });
        }
    }
    rows
}
fn paint_line(
    page: &mut Page,
    faces: &Faces,
    spans: &[Span],
    mut x: f64,
    y: f64,
    size: f64,
    bold: bool,
) {
    for s in spans {
        let color = if s.href.is_some() {
            [21, 92, 132, 255]
        } else {
            [23, 31, 40, 255]
        };
        text(
            page,
            Some(faces),
            &s.text,
            &Setting {
                size,
                at: [x, y],
                centred: false,
                angle: 0.0,
                italic: s.italic,
                color,
                halo: None,
                heavy: bold || s.bold,
            },
        );
        let width = f64::from(crate::scene::text_width(&s.text, size as f32));
        if s.strike || s.href.is_some() {
            let line_y = if s.strike { y - size * 0.30 } else { y + 2.0 };
            page.stroke(&crate::scene::SceneLine {
                color,
                pts: vec![[x, line_y], [x + width, line_y]],
                width: 0.8,
                dash: None,
            });
        }
        x += width;
    }
}
struct Flow<'a> {
    sheets: Vec<Sheet>,
    faces: &'a Faces<'a>,
    images: &'a BTreeMap<u64, ImageRef>,
}
impl Flow<'_> {
    fn room(&mut self, height: f64) -> &mut Sheet {
        if self.sheets.last().unwrap().y + height > H - M && self.sheets.last().unwrap().y > M {
            self.sheets.push(Sheet::new());
        }
        self.sheets.last_mut().unwrap()
    }
    fn paragraph(&mut self, kind: Paragraph, spans: &[Span]) {
        let size = match kind {
            Paragraph::Heading(1) => 30.0,
            Paragraph::Heading(2) => 23.0,
            Paragraph::Heading(_) => 19.0,
            _ => 15.0,
        };
        let left = if matches!(kind, Paragraph::Quote) {
            M + 24.0
        } else {
            M
        };
        let lines = layout_text(spans, size, W - M - left);
        for line in lines {
            let faces = self.faces;
            let sheet = self.room(size * 1.46);
            paint_line(
                &mut sheet.page,
                faces,
                &line,
                left,
                sheet.y + size,
                size,
                matches!(kind, Paragraph::Heading(_)),
            );
            sheet.y += size * 1.46;
        }
        self.sheets.last_mut().unwrap().y += 8.0;
    }
    fn picture(&mut self, id: u64) -> bool {
        let Some(&(_, w, h)) = self.images.get(&id) else {
            return false;
        };
        let scale = ((W - 2.0 * M) / f64::from(w))
            .min(460.0 / f64::from(h))
            .min(1.0);
        let (w, h) = (f64::from(w) * scale, f64::from(h) * scale);
        let sheet = self.room(h + 18.0);
        let y = sheet.y + h;
        let _ = writeln!(
            sheet.page.ops,
            "q {w:.2} 0 0 -{h:.2} {M} {y:.2} cm /Im{id} Do Q"
        );
        sheet.used.insert(id);
        sheet.y += h + 14.0;
        true
    }
    fn table(&mut self, rows: &[Vec<Vec<Span>>]) {
        let n = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let col = (W - 2.0 * M) / n as f64;
        let size = 13.0;
        for (i, row) in rows.iter().enumerate() {
            let cells: Vec<_> = row
                .iter()
                .map(|line| layout_text(line, size, (col - 12.0).max(1.0)))
                .collect();
            let count = cells.iter().map(Vec::len).max().unwrap_or(1);
            // Very tall cells split over pages while keeping their columns.
            let max_lines = ((H - 2.0 * M - 15.0) / 20.0) as usize;
            for first in (0..count).step_by(max_lines) {
                let end = (first + max_lines).min(count);
                let h = (end - first) as f64 * 20.0 + 10.0;
                let faces = self.faces;
                let sheet = self.room(h);
                for j in 0..n {
                    let x = M + col * j as f64;
                    let y = sheet.y;
                    let points = vec![[x, y], [x + col, y], [x + col, y + h], [x, y + h]];
                    sheet.page.fill(
                        &[points.clone()],
                        if i == 0 {
                            [226, 235, 242, 255]
                        } else {
                            [255, 255, 255, 255]
                        },
                    );
                    let mut outline = points;
                    outline.push(outline[0]);
                    sheet.page.stroke(&crate::scene::SceneLine {
                        pts: outline,
                        width: 0.75,
                        color: [157, 169, 181, 255],
                        dash: None,
                    });
                    if let Some(cell) = cells.get(j) {
                        for (k, line) in cell.iter().skip(first).take(end - first).enumerate() {
                            paint_line(
                                &mut sheet.page,
                                faces,
                                line,
                                x + 6.0,
                                y + 17.0 + k as f64 * 20.0,
                                size,
                                i == 0,
                            );
                        }
                    }
                }
                sheet.y += h;
            }
        }
        self.sheets.last_mut().unwrap().y += 12.0;
    }
}
pub(crate) fn render(pubn: &Publication) -> Result<Vec<u8>, String> {
    let font = crate::png::find_system_font().ok_or("PDF 导出需要可用字体")?;
    let base = Face::parse(&font, 0).map_err(|_| "无法读取 PDF 字体")?;
    let fallback = crate::png::symbol_font();
    let faces = Faces {
        list: std::iter::once(base)
            .chain(fallback.and_then(|b| Face::parse(b, 0).ok()))
            .collect(),
    };
    let mut pdf = Pdf {
        objects: vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), vec![]],
    };
    let mut images = BTreeMap::new();
    for (&id, asset) in &pubn.assets {
        if let Ok(image) = image::load_from_memory(&asset.bytes.0) {
            let thin = if image.width() > 1600 || image.height() > 1600 {
                image.thumbnail(1600, 1600)
            } else {
                image
            };
            let rgb = thin.to_rgb8();
            let (w, h) = rgb.dimensions();
            let extra=format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8");
            let n = pdf.add(Pdf::stream(rgb.as_raw(), &extra));
            images.insert(id, (n, w, h));
        }
    }
    let mut flow = Flow {
        sheets: vec![Sheet::new()],
        faces: &faces,
        images: &images,
    };
    for block in &pubn.blocks {
        match block {
            Block::Rule => {
                let sheet = flow.room(17.0);
                sheet.page.stroke(&crate::scene::SceneLine {
                    pts: vec![[M, sheet.y + 4.0], [W - M, sheet.y + 4.0]],
                    width: 1.0,
                    color: [178, 186, 190, 255],
                    dash: None,
                });
                sheet.y += 16.0;
            }
            Block::Table(rows) => flow.table(rows),
            Block::Para(kind, spans) => {
                let mut pending = Vec::new();
                for span in spans {
                    if span.image {
                        if let Some(id) = span.href.as_deref().and_then(asset_id) {
                            if images.contains_key(&id) {
                                if !pending.is_empty() {
                                    flow.paragraph(*kind, &pending);
                                    pending.clear();
                                }
                                flow.picture(id);
                                if !span.text.is_empty() {
                                    flow.paragraph(
                                        Paragraph::Quote,
                                        &[Span {
                                            text: span.text.clone(),
                                            italic: true,
                                            ..Default::default()
                                        }],
                                    );
                                }
                                continue;
                            }
                        }
                    }
                    pending.push(span.clone());
                }
                if !pending.is_empty() {
                    flow.paragraph(*kind, &pending);
                }
            }
        }
    }
    let mut pages = Vec::new();
    for sheet in flow.sheets {
        let contents = pdf.add(Pdf::stream(sheet.page.ops.as_bytes(), ""));
        let images = sheet
            .used
            .iter()
            .map(|id| format!("/Im{id} {} 0 R ", images[id].0))
            .collect::<String>();
        let alpha = sheet
            .page
            .alphas
            .iter()
            .map(|(a, name)| {
                format!(
                    "/{name} << /ca {:.3} /CA {:.3} >> ",
                    f64::from(*a) / 1000.0,
                    f64::from(*a) / 1000.0
                )
            })
            .collect::<String>();
        let page=pdf.add(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] /Resources << /XObject << {images}>> /ExtGState << {alpha}>> >> /Contents {contents} 0 R >>",W*PT,H*PT));
        pages.push(format!("{page} 0 R"));
    }
    pdf.objects[1] = format!(
        "<< /Type /Pages /Count {} /Kids [{}] >>",
        pages.len(),
        pages.join(" ")
    )
    .into_bytes();
    Ok(pdf.finish())
}
