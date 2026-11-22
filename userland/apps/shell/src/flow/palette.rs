//! The colours of Flow's album cards, grown by Detent's `bezel` generator
//! from the same 72 seeds as "your own colour" (`bezel::hues`): for each,
//! the two ends of Bezel's case gradient (the seed at signal chroma, tones 64
//! and 52), its signal, and its dark ground. Written by
//! `scripts/grow-flow-cards.mjs`.

/// (case gradient start, case gradient end, signal, ground), every 5°.
pub const CARDS: [(u32, u32, u32, u32); 72] = [
    // hue 0°, seed #eb4242
    (0xff6b65, 0xe43d3e, 0xff605b, 0x140c0b),
    // hue 5°, seed #eb5042
    (0xff6c5c, 0xdd4639, 0xff6151, 0x140c0b),
    // hue 10°, seed #eb5e42
    (0xff6d50, 0xd64f34, 0xf8684b, 0x140c0a),
    // hue 15°, seed #eb6c42
    (0xf7754a, 0xce572f, 0xf07045, 0x140c0a),
    // hue 20°, seed #eb7a42
    (0xee7b43, 0xc65e27, 0xe7763e, 0x140c09),
    // hue 25°, seed #eb8842
    (0xe97f30, 0xc26110, 0xe37a2b, 0x140d08),
    // hue 30°, seed #eb9642
    (0xe28418, 0xb86800, 0xdc7f11, 0x130d07),
    // hue 35°, seed #eba442
    (0xd78b00, 0xad6e00, 0xd08600, 0x130d07),
    // hue 40°, seed #ebb242
    (0xca9100, 0xa27400, 0xc48c00, 0x120e07),
    // hue 45°, seed #ebc042
    (0xbf9600, 0x997800, 0xb89100, 0x110e07),
    // hue 50°, seed #ebce42
    (0xb49a00, 0x907b00, 0xae9500, 0x100e07),
    // hue 55°, seed #ebdd42
    (0xa99e00, 0x887e00, 0xa49800, 0x0f0e07),
    // hue 60°, seed #ebeb42
    (0xa0a000, 0x808000, 0x9b9b00, 0x0f0f07),
    // hue 65°, seed #ddeb42
    (0x98a300, 0x798200, 0x929d00, 0x0e0f08),
    // hue 70°, seed #ceeb42
    (0x8ea500, 0x718400, 0x89a000, 0x0e0f08),
    // hue 75°, seed #c0eb42
    (0x84a800, 0x698600, 0x7fa200, 0x0d0f08),
    // hue 80°, seed #b2eb42
    (0x79aa00, 0x608800, 0x75a400, 0x0d0f09),
    // hue 85°, seed #a4eb42
    (0x6eac00, 0x578a00, 0x6aa600, 0x0d0f09),
    // hue 90°, seed #96eb42
    (0x62ae00, 0x4d8b00, 0x5ea800, 0x0c0f09),
    // hue 95°, seed #88eb42
    (0x55b000, 0x438d00, 0x52aa00, 0x0c0f09),
    // hue 100°, seed #7aeb42
    (0x46b100, 0x378e00, 0x44ab00, 0x0c0f09),
    // hue 105°, seed #6ceb42
    (0x36b200, 0x298f00, 0x34ac00, 0x0c0f0a),
    // hue 110°, seed #5eeb42
    (0x1fb400, 0x179000, 0x1eae00, 0x0c0f0a),
    // hue 115°, seed #50eb42
    (0x00b40a, 0x009105, 0x00ae09, 0x0b0f0a),
    // hue 120°, seed #42eb42
    (0x00b41c, 0x009014, 0x00ae1b, 0x0b0f0a),
    // hue 125°, seed #42eb50
    (0x00b42c, 0x009021, 0x00ae2a, 0x0b0f0a),
    // hue 130°, seed #42eb5e
    (0x00b43a, 0x00902d, 0x00ae38, 0x0b0f0a),
    // hue 135°, seed #42eb6c
    (0x00b347, 0x009037, 0x00ad44, 0x0b0f0b),
    // hue 140°, seed #42eb7a
    (0x00b353, 0x008f41, 0x00ad50, 0x0b0f0b),
    // hue 145°, seed #42eb88
    (0x00b25f, 0x008f4b, 0x00ac5c, 0x0a0f0b),
    // hue 150°, seed #42eb96
    (0x00b26b, 0x008f55, 0x00ac67, 0x0a0f0c),
    // hue 155°, seed #42eba4
    (0x00b176, 0x008e5e, 0x00ab72, 0x0a0f0c),
    // hue 160°, seed #42ebb2
    (0x00b081, 0x008d67, 0x00aa7d, 0x0a0f0d),
    // hue 165°, seed #42ebc0
    (0x00b08c, 0x008d70, 0x00aa88, 0x090f0d),
    // hue 170°, seed #42ebce
    (0x00af97, 0x008c79, 0x00a992, 0x090f0e),
    // hue 175°, seed #42ebdd
    (0x00aea3, 0x008b82, 0x00a89d, 0x090f0f),
    // hue 180°, seed #42ebeb
    (0x00adad, 0x008a8b, 0x00a7a7, 0x090f0f),
    // hue 185°, seed #42ddeb
    (0x00acb8, 0x008993, 0x00a6b2, 0x090f10),
    // hue 190°, seed #42ceeb
    (0x00aac5, 0x00889e, 0x00a4be, 0x090f11),
    // hue 195°, seed #42c0eb
    (0x00a8d3, 0x0087aa, 0x00a3cc, 0x0a0f11),
    // hue 200°, seed #42b2eb
    (0x00a6e4, 0x0085b7, 0x00a0dc, 0x0a0f12),
    // hue 205°, seed #42a4eb
    (0x2aa2f0, 0x0082c6, 0x1f9dea, 0x0b0f12),
    // hue 210°, seed #4296eb
    (0x4a9ff6, 0x197fd4, 0x4399f0, 0x0b0e13),
    // hue 215°, seed #4288eb
    (0x599bff, 0x317bdd, 0x5296fa, 0x0c0e13),
    // hue 220°, seed #427aeb
    (0x6b98ff, 0x3f77e8, 0x6192ff, 0x0c0e13),
    // hue 225°, seed #426ceb
    (0x7796ff, 0x4a73f2, 0x6f90ff, 0x0d0e13),
    // hue 230°, seed #425eeb
    (0x8093ff, 0x536ffc, 0x798dff, 0x0d0e13),
    // hue 235°, seed #4250eb
    (0x8792ff, 0x5d6bff, 0x808bff, 0x0e0e13),
    // hue 240°, seed #4242eb
    (0x8d90ff, 0x6669ff, 0x868aff, 0x0e0e13),
    // hue 245°, seed #5042eb
    (0x938eff, 0x6f66ff, 0x8d88ff, 0x0e0e13),
    // hue 250°, seed #5e42eb
    (0x9a8cff, 0x7963ff, 0x9485ff, 0x0e0e13),
    // hue 255°, seed #6c42eb
    (0xa289ff, 0x835eff, 0x9d82ff, 0x0f0d13),
    // hue 260°, seed #7a42eb
    (0xaa86ff, 0x8e59ff, 0xa57fff, 0x0f0d13),
    // hue 265°, seed #8842eb
    (0xb382ff, 0x9854fc, 0xae7bff, 0x0f0d12),
    // hue 270°, seed #9642eb
    (0xbc7eff, 0xa250f8, 0xb876ff, 0x100d12),
    // hue 275°, seed #a442eb
    (0xc679ff, 0xac4af3, 0xc271ff, 0x100d12),
    // hue 280°, seed #b242eb
    (0xd073ff, 0xb545ee, 0xcd6aff, 0x110d12),
    // hue 285°, seed #c042eb
    (0xda6cff, 0xbd3fe8, 0xd762ff, 0x110d12),
    // hue 290°, seed #ce42eb
    (0xe563ff, 0xc538e2, 0xe357ff, 0x110d11),
    // hue 295°, seed #dd42eb
    (0xf257fe, 0xcd2fdc, 0xeb51f9, 0x120d11),
    // hue 300°, seed #eb42eb
    (0xf950f8, 0xd326d5, 0xf34af2, 0x120d11),
    // hue 305°, seed #eb42dd
    (0xfc52ec, 0xd52aca, 0xf54ce6, 0x120c11),
    // hue 310°, seed #eb42ce
    (0xfe54df, 0xd72ebd, 0xf84ed9, 0x120c10),
    // hue 315°, seed #eb42c0
    (0xff58d2, 0xd931b1, 0xfa50cd, 0x130c10),
    // hue 320°, seed #eb42b2
    (0xff5bc4, 0xdb33a5, 0xfc51c0, 0x130c10),
    // hue 325°, seed #eb42a4
    (0xff5fb7, 0xdd3598, 0xfe52b4, 0x130c0f),
    // hue 330°, seed #eb4296
    (0xff61aa, 0xde378c, 0xff54a6, 0x130c0f),
    // hue 335°, seed #eb4288
    (0xff649e, 0xdf387f, 0xff5799, 0x140c0e),
    // hue 340°, seed #eb427a
    (0xff6692, 0xe13a72, 0xff5a8c, 0x140c0e),
    // hue 345°, seed #eb426c
    (0xff6887, 0xe23b65, 0xff5c80, 0x140c0d),
    // hue 350°, seed #eb425e
    (0xff697b, 0xe33c58, 0xff5d73, 0x140c0d),
    // hue 355°, seed #eb4250
    (0xff6a70, 0xe33c4b, 0xff5f67, 0x140c0c),
];
