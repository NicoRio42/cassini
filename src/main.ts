import sharp from "sharp";

const buffer = await sharp("out/slopes.tif").threshold(40).raw().toBuffer();

//   .toFile("out/slope.png");

for (let i = 0; i < buffer.length; i++) {
  if (buffer[i] === 0) console.log(buffer[i]);
}
