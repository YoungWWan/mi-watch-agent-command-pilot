import os
import re

def clean_nanopb_tags_from_proto(file_path):
    with open(file_path, 'r', encoding='utf-8') as f:
        content = f.read()

    content = re.sub(r'^\s*import\s+"nanopb\.proto";\s*\n?', '', content, flags=re.MULTILINE)

    content = re.sub(r'\s*\[\s*\(nanopb\)\.[^\]]+\]', '', content)

    with open(file_path, 'w', encoding='utf-8') as f:
        f.write(content)

    print(f"Cleaned: {file_path}")

def main():
    for filename in os.listdir('.'):
        if filename.endswith('.proto'):
            clean_nanopb_tags_from_proto(filename)

if __name__ == "__main__":
    main()
