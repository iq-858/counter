with open('src/video.rs','r') as f:
    content = f.read()
# Find the second decode_loop definition (old body with cfg) and comment it out
parts = content.split('fn decode_loop(slot: Arc<VideoSlot>, name:')
if len(parts) == 3:
    # parts[0] before first; parts[1] windows version; parts[2] rest (includes old)
    # We want to keep first (now default) and remove old at end
    # Find where second "fn decode_loop" starts in parts[2]
    rest = parts[2]
    idx = rest.find('fn decode_loop')
    if idx != -1:
        # Keep everything before that index (which is just bottom helpers after old loop)
        # Actually rest starts with old decode_loop; everything after it is bottom helpers
        # We need to delete from idx to end of old loop, but keep bottom helpers after loop
        # The loop ends with "    sleep(frame_interval);\n    }\n}\n" before helpers
        loop_end = rest.find('    sleep(frame_interval);\n    }\n}\n')
        if loop_end != -1:
            # Keep from loop_end+len after loop close onward
            bottom = rest[loop_end + len('    sleep(frame_interval);\n    }\n}\n'):]
            # But bottom includes more helpers; just reconstruct
            new_rest = bottom
            content = parts[0] + 'fn decode_loop(slot: Arc<VideoSlot>, name: ' + parts[1] + new_rest
with open('src/video.rs','w') as f:
    f.write(content)
