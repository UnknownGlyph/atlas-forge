use super::{asset_id, escape, is_web, Block, Paragraph, Publication, Span};
use std::io::{Cursor, Write};
use zip::{write::SimpleFileOptions, ZipWriter};

const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PACKAGE: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
struct Word<'a> {
    publication: &'a Publication,
    relations: String,
    pictures: Vec<Vec<u8>>,
    next: usize,
}
impl Word<'_> {
    fn relation(&mut self, kind: &str, target: &str, external: bool) -> usize {
        self.next += 1;
        self.relations.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"{OFFICE}/{kind}\" Target=\"{}\"{}/>",
            self.next,
            escape(target),
            if external {
                " TargetMode=\"External\""
            } else {
                ""
            }
        ));
        self.next
    }
    fn drawing(&mut self, s: &Span) -> Option<String> {
        use image::GenericImageView;
        let a = self
            .publication
            .assets
            .get(&asset_id(s.href.as_deref()?)?)?;
        let mut img = image::load_from_memory(&a.bytes.0).ok()?;
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return None;
        }
        if w > 1800 || h > 1800 {
            img = img.thumbnail(1800, 1800);
        }
        let mut bytes = Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).ok()?;
        let n = self.pictures.len() + 1;
        self.pictures.push(bytes.into_inner());
        let id = self.relation("image", &format!("media/picture{n}.png"), false);
        let scale = (550.0 / f64::from(w)).min(460.0 / f64::from(h)).min(1.0);
        let (cx, cy) = (
            (f64::from(w) * scale * 9525.0) as u64,
            (f64::from(h) * scale * 9525.0) as u64,
        );
        Some(format!(
            r#"<w:r><w:drawing><wp:inline><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{n}" name="Picture {n}" descr="{}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{n}" name="Picture {n}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId{id}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#,
            escape(&s.text)
        ))
    }
    fn span(&mut self, s: &Span, kind: Paragraph) -> String {
        if s.image {
            if let Some(image) = self.drawing(s) {
                return image;
            }
        }
        let value = if s.image {
            format!("[图片：{}]", s.text)
        } else {
            s.text.clone()
        };
        let size = match kind {
            Paragraph::Heading(1) => 40,
            Paragraph::Heading(2) => 32,
            Paragraph::Heading(_) => 27,
            _ => 22,
        };
        let props=format!("<w:rPr><w:rFonts w:ascii=\"{}\" w:eastAsia=\"Microsoft YaHei\"/><w:sz w:val=\"{size}\"/>{}{}{}{}</w:rPr>",
            if s.code{"Consolas"}else{"Calibri"},if s.bold||matches!(kind,Paragraph::Heading(_)){"<w:b/>"}else{""},
            if s.italic{"<w:i/>"}else{""},if s.strike{"<w:strike/>"}else{""},if s.href.is_some(){"<w:color w:val=\"17618D\"/><w:u w:val=\"single\"/>"}else{""});
        let content = value
            .split('\n')
            .map(|t| format!("<w:t xml:space=\"preserve\">{}</w:t>", escape(t)))
            .collect::<Vec<_>>()
            .join("<w:br/>");
        let text = format!("<w:r>{props}{content}</w:r>");
        if let Some(href) = s.href.as_deref().filter(|h| is_web(h)) {
            let id = self.relation("hyperlink", href, true);
            return format!("<w:hyperlink r:id=\"rId{id}\">{text}</w:hyperlink>");
        }
        if let Some(id) = s
            .href
            .as_deref()
            .and_then(|uri| self.publication.destination(uri))
        {
            return format!("<w:hyperlink w:anchor=\"doc_{id}\">{text}</w:hyperlink>");
        }
        text
    }
    fn paragraph(&mut self, kind: Paragraph, spans: &[Span], bookmark: Option<&u64>) -> String {
        let ppr=match kind{
            Paragraph::Heading(level)=>format!("<w:pPr><w:keepNext/><w:spacing w:before=\"210\" w:after=\"110\"/><w:outlineLvl w:val=\"{}\"/></w:pPr>",level.saturating_sub(1)),
            Paragraph::Quote=>"<w:pPr><w:ind w:left=\"450\"/></w:pPr>".into(),
            _=>"<w:pPr><w:spacing w:after=\"110\"/></w:pPr>".into(),
        };
        let text = spans.iter().map(|s| self.span(s, kind)).collect::<String>();
        if let Some(id) = bookmark {
            format!("<w:p>{ppr}<w:bookmarkStart w:id=\"{id}\" w:name=\"doc_{id}\"/>{text}<w:bookmarkEnd w:id=\"{id}\"/></w:p>")
        } else {
            format!("<w:p>{ppr}{text}</w:p>")
        }
    }
}
pub(super) fn docx(pubn: &Publication) -> Result<Vec<u8>, String> {
    let mut doc = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
    use std::fmt::Write as _;
    let _ = write!(
        doc,
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="{OFFICE}" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>"#
    );
    let mut word = Word {
        publication: pubn,
        relations: String::new(),
        pictures: Vec::new(),
        next: 0,
    };
    for (index, b) in pubn.blocks.iter().enumerate() {
        match b {
            Block::Para(kind,spans)=>doc.push_str(&word.paragraph(*kind,spans,pubn.anchors.get(&index))),
            Block::Rule=>doc.push_str(r#"<w:p><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="6" w:space="4" w:color="A0A8B2"/></w:pBdr></w:pPr></w:p>"#),
            Block::Table(rows)=>{
                doc.push_str(r#"<w:tbl><w:tblPr><w:tblW w:type="auto" w:w="0"/><w:tblBorders><w:top w:val="single" w:sz="4"/><w:left w:val="single" w:sz="4"/><w:bottom w:val="single" w:sz="4"/><w:right w:val="single" w:sz="4"/><w:insideH w:val="single" w:sz="4"/><w:insideV w:val="single" w:sz="4"/></w:tblBorders></w:tblPr><w:tblGrid>"#);
                let n=rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                for _ in 0..n {let _=write!(doc,"<w:gridCol w:w=\"{}\"/>",9200/n);}
                doc.push_str("</w:tblGrid>");
                for (i,row) in rows.iter().enumerate(){
                    doc.push_str("<w:tr>");
                    for cell in row{
                        doc.push_str("<w:tc>");
                        if i==0{doc.push_str("<w:tcPr><w:shd w:fill=\"E3EBF2\"/></w:tcPr>");}
                        doc.push_str(&word.paragraph(Paragraph::Text,cell,None));doc.push_str("</w:tc>");
                    }
                    doc.push_str("</w:tr>");
                }
                doc.push_str("</w:tbl>");
            }
        }
    }
    doc.push_str(r#"<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1100" w:bottom="1100" w:left="1300" w:right="1300"/></w:sectPr></w:body></w:document>"#);
    let mut package = ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, value: &[u8]| -> Result<(), String> {
        package
            .start_file(name, SimpleFileOptions::default())
            .map_err(|e| e.to_string())?;
        package.write_all(value).map_err(|e| e.to_string())
    };
    add("[Content_Types].xml",br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#)?;
    add("_rels/.rels",format!(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{PACKAGE}"><Relationship Id="rId1" Type="{OFFICE}/officeDocument" Target="word/document.xml"/></Relationships>"#).as_bytes())?;
    add("word/document.xml", doc.as_bytes())?;
    if !word.relations.is_empty() {
        add("word/_rels/document.xml.rels",format!(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{PACKAGE}">{}</Relationships>"#,word.relations).as_bytes())?;
    }
    for (i, img) in word.pictures.iter().enumerate() {
        add(&format!("word/media/picture{}.png", i + 1), img)?;
    }
    Ok(package.finish().map_err(|e| e.to_string())?.into_inner())
}
