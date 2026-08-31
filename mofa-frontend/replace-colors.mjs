import fs from 'fs';
import path from 'path';

const searchDir = './src';

const replacements = [
  // Backgrounds
  { regex: /bg-white(?!(\/|\w))/g, replacement: 'bg-background-card' },
  { regex: /bg-black\/5/g, replacement: 'bg-background-hover' },
  { regex: /bg-black\/\[0\.02\]/g, replacement: 'bg-background-hover' },
  { regex: /bg-black\/10/g, replacement: 'bg-white/5' },
  { regex: /bg-black\/20/g, replacement: 'bg-white/10' },
  { regex: /bg-black\/40/g, replacement: 'bg-background-secondary' },
  
  // Borders
  { regex: /border-black\/5/g, replacement: 'border-border-subtle' },
  { regex: /border-black\/\[0\.03\]/g, replacement: 'border-border-subtle' },
  { regex: /border-black\/10/g, replacement: 'border-border-strong' },
  { regex: /border-border-light/g, replacement: 'border-border-subtle' },
  { regex: /border-white\/10/g, replacement: 'border-border-strong' },
  { regex: /border-white\/5/g, replacement: 'border-border-subtle' },
  
  // Divides
  { regex: /divide-black\/5/g, replacement: 'divide-border-subtle' },
  { regex: /divide-black\/10/g, replacement: 'divide-border-strong' },
  
  // Specific exceptions in Button.tsx
  { regex: /hover:bg-gray-50/g, replacement: 'hover:bg-background-hover' },
];

function walk(dir) {
  let results = [];
  const list = fs.readdirSync(dir);
  list.forEach(file => {
    file = path.join(dir, file);
    const stat = fs.statSync(file);
    if (stat && stat.isDirectory()) {
      results = results.concat(walk(file));
    } else if (file.endsWith('.tsx') || file.endsWith('.ts')) {
      results.push(file);
    }
  });
  return results;
}

const files = walk(searchDir);
let changedFiles = 0;

files.forEach(file => {
  let content = fs.readFileSync(file, 'utf8');
  let originalContent = content;
  
  replacements.forEach(({ regex, replacement }) => {
    content = content.replace(regex, replacement);
  });
  
  if (content !== originalContent) {
    fs.writeFileSync(file, content, 'utf8');
    changedFiles++;
    console.log(`Updated ${file}`);
  }
});

console.log(`\nSuccessfully updated ${changedFiles} files for Dark Mode compatibility.`);
