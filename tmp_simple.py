with open('src/video.rs','r') as f:
    lines = f.read().splitlines()
start = None
end = None
for i,ln in enumerate(lines):
    if ln.startswith('#[cfg(not(windows))]') and i+1 < len(lines) and 'fn decode_loop' in lines[i+1]:
        start = i
    if start is not None and ln.startswith('#[cfg(not(windows))]') and i > start and 'struct TrackInfo' in lines[i+1] if i+1 < len(lines) else False:
        end = i
        break
    # Also stop if we see struct TrackInfo line after loop (without cfg)
    if start is not None and ln.startswith('struct TrackInfo'):
        end = i
        break
if start is not None and end is not None:
    new_lines = lines[:start] + lines[end:]
    with open('src/video.rs','w') as f:
        f.write('\n'.join(new_lines)+'\n')
    print('Removed old loop from', start, 'to', end)
else:
    print('Not found', start, end)
