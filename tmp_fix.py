with open('src/mf.rs','r') as f:
    lines = f.read().splitlines()
lines.insert(40, '        unsafe {')
for i,ln in enumerate(lines):
    if ln.strip() == 'Ok(MfDecoder { reader, width, height, fps, stride, video_stream })':
        lines.insert(i+1, '        }')
        break
with open('src/mf.rs','w') as f:
    f.write('\n'.join(lines)+'\n')
