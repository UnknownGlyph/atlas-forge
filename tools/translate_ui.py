"""Build offline message catalogs from the UI inventory.

Needs CTranslate2, sentencepiece, and OpenCC on the Python path, plus
unpacked Argos models under the supplied models directory. Existing
translations are retained so reviewers can edit the JSON directly.
"""
import argparse
import json
import re
from pathlib import Path
from collections import Counter
from concurrent.futures import ThreadPoolExecutor

import ctranslate2
import sentencepiece
from opencc import OpenCC

ROOT = Path(__file__).resolve().parent.parent
TARGETS = ['ja','ru','de','fr','it','es','pt']
TOKENS = re.compile(r'\{[^{}]*\}|https?://[^\s)<>，。]+')

HUMAN = {
    '地块':'Face', '画板':'Artboard', '资料':'Library', '属性':'Properties',
    '草稿':'Draft','北回归线':'Tropic of Cancer', '相互引力':'Mutual gravity',
    '太阳邻域':'Solar neighborhood','无模型':'No model',
    '添加自定义天体':'Add custom body','添加伴星':'Add companion star',
    '通用助手':'General assistant','绘图助手':'Map assistant',
    '地月系':'Planet–Moon system','本恒星系':'Local star system',
    '整体移动陆块':'Move land block','整体缩放陆块':'Scale land block',
    '锁定':'Lock','导入 3D 模型':'Import 3D model','打开 3D 查看器':'Open 3D viewer',
    '相邻没有领土重叠':'Adjacent territories do not overlap',
}


def english_table():
    text = (ROOT/'crates/map_app/src/i18n.rs').read_text(encoding='utf8')
    quoted = r'"(?:[^"\\]|\\.)*"'
    pairs = re.findall(r'\(\s*('+quoted+r')\s*,\s*('+quoted+r')\s*\)',text)
    result = {}
    for a,b in pairs:
        try:
            result[json.loads(a)] = json.loads(b)
        except ValueError:
            pass
    result.update(HUMAN)
    return result


def write_json(path,value):
    pending = path.with_suffix('.writing')
    pending.write_text(json.dumps(value,ensure_ascii=False,sort_keys=True,indent=2)+'\n',encoding='utf8')
    pending.replace(path)


def read_json(path):
    try:
        return json.loads(path.read_text(encoding='utf8'))
    except FileNotFoundError:
        return {}


def translate(source, model, threads):
    if not source: return []
    token = sentencepiece.SentencePieceProcessor(model_file=str(model/'sentencepiece.model'))
    backend = ctranslate2.Translator(str(model/'model'),device='cpu',compute_type='int8',
                                    intra_threads=threads,inter_threads=1)

    def infer(text):
        out = []
        for off in range(0,len(text),80):
            batch = text[off:off+80]
            runs = backend.translate_batch(token.encode(batch,out_type=str),max_batch_size=256,
                                          batch_type='tokens',beam_size=3,
                                          max_decoding_length=268,replace_unknowns=True)
            out.extend(token.decode(r.hypotheses[0]).replace('▁', ' ') for r in runs)
        return out

    flat = list(dict.fromkeys(line for message in source for line in message.split('\n') if line.strip()))
    skeleton = []
    tags = []
    for line in flat:
        held = TOKENS.findall(line)
        substitutions = iter(range(len(held)))
        skeleton.append(TOKENS.sub(lambda _:f' [V{next(substitutions)}] ',line).strip())
        tags.append(held)
    results = infer(skeleton)
    errors = []
    texts = {}
    for i,(line, result, held) in enumerate(zip(flat,results,tags)):
        found = re.findall(r'\[\s*[Vv]\s*(\d+)\s*\]',result)
        if len(found)!=len(held) or len(set(found))!=len(held) or any(int(n)>=len(held) for n in found):
            errors.append(i)
        else:
            texts[line] = re.sub(r'\[\s*[Vv]\s*(\d+)\s*\]',lambda m:held[int(m[1])],result)
    # If an older model drops a sentinel, translate the phrases on either
    # side and keep every protected token in the resulting sentence.
    groups = {}
    for i in errors:
        line = flat[i]
        parts = []
        pos = 0
        for match in TOKENS.finditer(line):
            parts.extend([(line[pos:match.start()], False),(match.group(),True)])
            pos = match.end()
        parts.append((line[pos:],False))
        groups[i] = parts
    isolated = list(dict.fromkeys(s.strip() for items in groups.values() for s,is_token in items if not is_token and s.strip()))
    fragments = dict(zip(isolated,infer(isolated)))
    for i,parts in groups.items():
        texts[flat[i]] = ' '.join((s if protected else fragments[s.strip()]) for s,protected in parts if s.strip())

    def clean(original):
        if not original.strip(): return original
        output = texts[original].strip()
        if not output or Counter(TOKENS.findall(original)) != Counter(TOKENS.findall(output)):
            return original
        leading = original[:len(original)-len(original.lstrip())]
        trailing = original[len(original.rstrip()):]
        return leading+output+trailing
    return ['\n'.join(clean(line) for line in message.split('\n')) for message in source]


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('inventory',type=Path)
    cli.add_argument('models',type=Path)
    cli.add_argument('--threads',type=int,default=6)
    args=cli.parse_args()
    input_keys = read_json(args.inventory)
    locale_dir=ROOT/'assets/i18n'
    verified=english_table()
    all_keys=sorted(set(input_keys)|verified.keys())
    en=read_json(locale_dir/'en.json')
    en.update(verified)
    missing=[k for k in all_keys if k not in en]
    en.update(zip(missing,translate(missing,args.models/'zh-en',args.threads)))
    write_json(locale_dir/'en.json',en)
    print('en',len(en),'messages',flush=True)
    cc=OpenCC('s2twp')
    tw=read_json(locale_dir/'zh-TW.json')
    tw.update({k:cc.convert(k) for k in all_keys if k not in tw})
    write_json(locale_dir/'zh-TW.json',tw)
    print('zh-TW',len(tw),'messages',flush=True)

    def build(lang):
        path=locale_dir/(lang+'.json')
        data=read_json(path)
        keys=[k for k in all_keys if k not in data]
        needs=list(dict.fromkeys(en[k] for k in keys))
        translated=dict(zip(needs,translate(needs,args.models/('en-'+lang),max(1,args.threads//2))))
        data.update({k:translated[en[k]] for k in keys})
        write_json(path,data)
        print(lang,len(data),'messages',flush=True)

    with ThreadPoolExecutor(max_workers=2) as pool:
        list(pool.map(build,TARGETS))


if __name__=='__main__':
    main()
