with open(".Jules/bolt.md", "r") as f:
    lines = f.readlines()

new_lines = []
for line in lines:
    if line.startswith("<<<<<<< HEAD"):
        continue
    if line.startswith("======="):
        continue
    if line.startswith(">>>>>>> origin/master"):
        continue
    if "\\n" in line:
        line = line.replace("\\n", "\n")
    new_lines.append(line)

with open(".Jules/bolt.md", "w") as f:
    f.writelines(new_lines)
