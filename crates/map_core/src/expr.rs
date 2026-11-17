//! Tiny arithmetic expression language for derived indicators.
//!
//! Grammar: expr := term (('+'|'-') term)* ; term := unary (('*'|'/') unary)* ;
//! unary := '-' unary | atom ; atom := number | ident | ident '(' args ')' | '(' expr ')'.
//! Functions: min, max, abs, sqrt, ln, log10, pow. Missing values propagate as None.

#[derive(Clone, Debug, PartialEq)]
pub enum Ast {
    Num(f64),
    Var(String),
    Neg(Box<Ast>),
    Bin(char, Box<Ast>, Box<Ast>),
    Call(String, Vec<Ast>),
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Sym(char),
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch.is_ascii_digit() || ch == '.' {
            let st = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.' || c[i] == 'e' || c[i] == 'E') {
                i += 1;
            }
            let t: String = c[st..i].iter().collect();
            out.push(Tok::Num(t.parse().map_err(|_| format!("无效数字 `{t}`"))?));
        } else if ch.is_alphabetic() || ch == '_' {
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(c[st..i].iter().collect()));
        } else if "+-*/(),".contains(ch) {
            out.push(Tok::Sym(ch));
            i += 1;
        } else {
            return Err(format!("无法识别的字符 `{ch}`"));
        }
    }
    Ok(out)
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(&Tok::Sym(c)) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Result<Ast, String> {
        let mut l = self.term()?;
        loop {
            if self.eat('+') {
                l = Ast::Bin('+', l.into(), self.term()?.into());
            } else if self.eat('-') {
                l = Ast::Bin('-', l.into(), self.term()?.into());
            } else {
                return Ok(l);
            }
        }
    }
    fn term(&mut self) -> Result<Ast, String> {
        let mut l = self.unary()?;
        loop {
            if self.eat('*') {
                l = Ast::Bin('*', l.into(), self.unary()?.into());
            } else if self.eat('/') {
                l = Ast::Bin('/', l.into(), self.unary()?.into());
            } else {
                return Ok(l);
            }
        }
    }
    fn unary(&mut self) -> Result<Ast, String> {
        if self.eat('-') {
            return Ok(Ast::Neg(self.unary()?.into()));
        }
        self.atom()
    }
    fn atom(&mut self) -> Result<Ast, String> {
        match self.peek().cloned() {
            Some(Tok::Num(v)) => {
                self.i += 1;
                Ok(Ast::Num(v))
            }
            Some(Tok::Ident(name)) => {
                self.i += 1;
                if self.eat('(') {
                    let mut args = vec![];
                    if !self.eat(')') {
                        loop {
                            args.push(self.expr()?);
                            if self.eat(')') {
                                break;
                            }
                            if !self.eat(',') {
                                return Err("函数参数缺少 `,` 或 `)`".into());
                            }
                        }
                    }
                    Ok(Ast::Call(name, args))
                } else {
                    Ok(Ast::Var(name))
                }
            }
            Some(Tok::Sym('(')) => {
                self.i += 1;
                let e = self.expr()?;
                if !self.eat(')') {
                    return Err("缺少 `)`".into());
                }
                Ok(e)
            }
            _ => Err("表达式不完整".into()),
        }
    }
}

pub fn parse(s: &str) -> Result<Ast, String> {
    let mut p = Parser { t: lex(s)?, i: 0 };
    let e = p.expr()?;
    if p.i != p.t.len() {
        return Err("表达式末尾有多余内容".into());
    }
    Ok(e)
}

pub fn vars(a: &Ast, out: &mut Vec<String>) {
    match a {
        Ast::Var(v) => out.push(v.clone()),
        Ast::Neg(x) => vars(x, out),
        Ast::Bin(_, l, r) => {
            vars(l, out);
            vars(r, out);
        }
        Ast::Call(_, args) => args.iter().for_each(|x| vars(x, out)),
        Ast::Num(_) => {}
    }
}

pub fn eval(a: &Ast, lookup: &dyn Fn(&str) -> Option<f64>) -> Option<f64> {
    let v = match a {
        Ast::Num(v) => *v,
        Ast::Var(k) => lookup(k)?,
        Ast::Neg(x) => -eval(x, lookup)?,
        Ast::Bin(op, l, r) => {
            let (l, r) = (eval(l, lookup)?, eval(r, lookup)?);
            match op {
                '+' => l + r,
                '-' => l - r,
                '*' => l * r,
                _ => {
                    if r == 0.0 {
                        return None;
                    }
                    l / r
                }
            }
        }
        Ast::Call(f, args) => {
            let v: Option<Vec<f64>> = args.iter().map(|x| eval(x, lookup)).collect();
            let v = v?;
            match (f.as_str(), v.as_slice()) {
                ("min", [a, b]) => a.min(*b),
                ("max", [a, b]) => a.max(*b),
                ("abs", [a]) => a.abs(),
                ("sqrt", [a]) => a.sqrt(),
                ("ln", [a]) => a.ln(),
                ("log10", [a]) => a.log10(),
                ("pow", [a, b]) => a.powf(*b),
                _ => return None,
            }
        }
    };
    v.is_finite().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn eval_basic() {
        let a = parse("gdp / pop * 1000 + -max(1, 2)").unwrap();
        let r = eval(&a, &|k| match k {
            "gdp" => Some(10.0),
            "pop" => Some(5.0),
            _ => None,
        });
        assert_eq!(r, Some(1998.0));
        assert_eq!(eval(&parse("x / 0").unwrap(), &|_| Some(1.0)), None);
        assert_eq!(eval(&parse("missing + 1").unwrap(), &|_| None), None);
        assert!(parse("1 +").is_err());
        assert!(parse("(1").is_err());
    }
}
