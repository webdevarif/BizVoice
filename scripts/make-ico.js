const { default: pngToIco } = require('png-to-ico');
const fs = require('fs');

pngToIco('public/logo.png')
  .then(buf => {
    fs.writeFileSync('assets/icon.ico', buf);
    console.log('ICO created:', buf.length, 'bytes');
  })
  .catch(err => console.error('Failed:', err.message));
