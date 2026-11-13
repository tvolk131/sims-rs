use sha2::{Digest, Sha224, Sha256, Sha384, Sha512, Sha512_224, Sha512_256, digest::Output};

const ABC: [&str; 6] = [
    "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7",
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
    "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
    "4634270f707b6a54daae7530460842e20e37ed265ceee9a43e8924aa",
    "53048e2681941ef99b2e29b76b4c7dabe4c2d0c634fc6d46e0e2f13107e7af23",
];

const PATTERNS: &[(usize, [&str; 6])] = &[
    (
        0,
        [
            "d14a028c2a3a2bc9476102bb288234c415a2b01f828ea62ac5b3e42f",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b",
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
            "6ed0dd02806fa89e25de060c19d3ac86cabb87d6a0ddd05c333b84f4",
            "c672b8d1ef56ed28ab87c3622c5114069bdd3ad7b8f9737498d0c01ecef0967a",
        ],
    ),
    (
        55,
        [
            "c6a93fabf6b9c69da008ed39b9b28bb104cdfa4244aae625eb355307",
            "2900465fcb533e05a158fd2b3be0e5e3b03740d83060aa3580e0d98a96bf2384",
            "5af70c3661eb084de491e3a9cc1d584c28c2e752933aa13f047f7d2cd5285607b9d9750373e59fc0b5ff84ccf9099738",
            "2fd061a12ae849786141611e02a18ad9dad5d1cf6980f67fd4021fe369e4fef874548e9e5146e53622a919b3dcbd652f9abdd9578c4bd0c8e28d38008ea922b5",
            "f8f97d56720455bf50f9baedb1a686a6f08dedaac5b04aa7c998c157",
            "37e5a79eaed1c7ac106cf3675a201b7fe070d0ce3ff6e57323daad7ebb3bc784",
        ],
    ),
    (
        56,
        [
            "f5b9574ed655e155c6990c76920bf76ebf5e97a7e059e971f43865c6",
            "31454ff48ef36af2f08fd511bdc37d9d5855ac23e992e5ff5445cb6b7674a674",
            "c1a6736be1fcc8891a626b08a10097fbf2fcce3acd19ec22154761c485f874ac8ca986a1c213b790ebb57adeb0008b52",
            "78ade6b0c755effd2b093d02bbeb8354d57c3e0c36457f941b47e2b384bc0a279c4cdaae1f9968e47bfa1e94438c33af1253067677087890f35f944a6fb8693e",
            "5a7957252fb00c897a4d2727b59bbb02eda683bdfc9fea38e3935854",
            "15afb3f08af95216a5babf4c3ee2144c119e2c7284ccbe67c67ebfc45dbce46f",
        ],
    ),
    (
        63,
        [
            "2a5214c3584c18d8010714ee27f7bdb640a5bd88b80ffc4e3c7d0b9b",
            "5f6401b96532c36de4e65beec0409b69b1d181864c8009b7a04f43e5d56350d1",
            "a4664ae68697964c5f1a1e4dde77f0237dab9e9ea7a6e5ceb2e50e984cdfad14bc55fd1c8830a5acff820e564a3c6a34",
            "853d2f48b4e8c74aa7c547ad831787b4f2bb85aa2851241a7fd1a38f651c2eb5816990092a4cd336d86dba73f0191f69a8825c7f5f2991bdcc2fb9661af22f46",
            "afe7123c8fffe81d0a402a30e5009153f4ff151d00204775ac8df02e",
            "b6e1ebb0da1dade8ed2920313d3add513cffa2ec6860f1a924176f9eca59b289",
        ],
    ),
    (
        64,
        [
            "a4887c68ee6f6191ee77160a3725ab626f1887ddc1462be04df2d8c6",
            "94eb5de4943613fd048dc93393ab06877405faa39c11f53e9386083339833e7e",
            "13c3b81071b4ff54bcb80912eeb97bfadc12c29df001f83defaf970a4e43b104aeda6a6e6fa71d0d2cc6c745f2952406",
            "be7cccdacc7551d40828f4f3568f741a224ec5e5274ca547e06ad5dc8313456706033259aa46937a061bc30f9913597e0b591c6492da5334e9148bbb91ce805b",
            "b4e317ca63b7c1a171961d387e310f0187d1ae9e86e1321c4734fdd5",
            "94b455d35e641fb88543f7dc795bd580562a9348deec2614c8637d036c772bee",
        ],
    ),
    (
        65,
        [
            "2bb05279298fe4f546d499d4befe9b794afb2c54bf1157c0a3c3f833",
            "fc518669b6eb4b4dd91827ecacef86689c725bd5bab888fd3b26dbb196eec954",
            "af669b8d0d69de941a5b881f960fcc4404b9cef88c62c5cb171212ba76fe44448eedc3c5c983b5552b745643ed16eeaa",
            "3be887ac920f6d448401e32d90815cbff19ec9b8042d9e14bd5d289db18437736f1a00b7573607684d83dc2f398ee82f5296beae0231e5bd2fe139b20292a18c",
            "b1c6324384952c04628bc0ecd0685f87a24bc95ca7e9685cd50b6e9f",
            "bbfe1d3d695242b8b0d42797abef1097b5b3a59f6153b939913d25fa9f6c532c",
        ],
    ),
    (
        111,
        [
            "870fadffe660fb09ef957e8652917d1472a43daf6dbfed58b4c7715d",
            "aeca4f5a02aead30b044df2ce00b97f2f4874c25352eaa1916a1831819dcc3dd",
            "d6517d210fc664af0e137a4ca19d7a299465109f40cc0fa9ac7f9d4ac9c77e6687f3cb060890ef7e96f9f5252dc528ab",
            "4d1db900250c96436052fbca79c13acbf378aad9c35b87d94c3803264df61fd22cbd327c8938d024db372abf4208934ee09367d571d6c670bf74ee07b83e7506",
            "a58fca1508a10042d2726853fd87baedc738d8c9353f05c18e3d917f",
            "84d71ab5b507d9440277cc05cdde25e57e396e3a880ee24b0009fade06bc4cd7",
        ],
    ),
    (
        112,
        [
            "76ae46b7b85cc695d4cb5c2718b2618e6a515aa7ee12ded60deae13a",
            "cec7a189fcea0a38f025f11208b1bd2b12ec291ff00ec5448a325c5c8db0c7ac",
            "76078618f7c580d9ca1f9990c2adb38a899f7feda86153dda507b72f1bbafb046ab835aa8d325a89a67e3deb81ad1ad5",
            "dfb715ca3478a894302ace39c42d1d6646e1044f2247a6274d8b42d155d2fdbe7017195e85cfba96bedc51f84c44638978a540039ff09c64cef6c0c5ccc8f7b6",
            "910acbee9104e54d812d15d96b7c0e6ece09a62b144c3ffec072fdc9",
            "ff0114959575adb2a8a59ddecba0e51855b385eb4a24017c923a0998062698f2",
        ],
    ),
    (
        127,
        [
            "d1f076cb6279edc842e2390c121b897e08b455d6048eb29966350da2",
            "0fe729ff19257bd6fec853acc2ea355f6b34b58e6c0f684c3e188fcdfcd9baae",
            "17ed2932ea5246073ae8240a06ec458b15bfd72d792520d0c319b84f97532d408fb7d2c39cd3710165914994f44a70f8",
            "f93a0e7465b294188e8aa2b1cc2e98bc8d5115d46f51c7a9ec599b9d9f96a80fef6a4f226b648c89bd9eac23b3d64264898b568d915c66666c44cd0319e2ef56",
            "7f5366f8c6bb99628f36036792f483cce175aaaa54031e4db1bda02a",
            "fceee8db128448e9d902b746eb7cc3fe08be09d0a08d7b7643d0a478a91a5568",
        ],
    ),
    (
        128,
        [
            "a690cfc89e8c061c8da6301144972af4be60dee97e8672ce7e52ef0c",
            "0aedd4856f8eba0963627336ad5144a9a7dbe12498e6066f0165fc97d8ddee4c",
            "13274b7eaabb6a3203608240e792e825ddb816853a66fde047b368cc7638d61b9f215323c4160677301ae4d178cd8579",
            "0b4815d35f9d07b1a30de2790e1be2a720234295cd7b4d9e9af51719ff90019f1fe6d4e402a7dcc4177085023dc460ab743dad9b2c1dda42662bda5d3b2e155b",
            "ed05e62e49b2d9345131438d9da5b9e764910ba482f503132abe92d9",
            "aa399434cc7ac88f2a667659396d5e9659ccbdc53d8f0d0c9ccd78dbfd970921",
        ],
    ),
    (
        129,
        [
            "8baa263ee929dd49b84e560cc7b79ba111f032a0c9216d279e675184",
            "4f1757ae4bffbae86d775b831765b75af154d52f7deaa46dd378051a2d3ad57f",
            "f07f92376b47fb9d34ff613a44f18b04b55fb1c480b07de4e08a0e9bd484ffa765c21671c341c70b4024e6bdcda709e4",
            "1809db04d02717483e04bc4333a14308bd2d0213ba7bf2c63f11eb1b8a0af8252e67fd104fd466fb95f945539824d8e4183155fa5ced0bee3dad46d9384a0bd5",
            "931c7f69b1d56b27c40b30923ebaf04355ac94b06ca0ff6658aef1b9",
            "345de9019055b4b8037c75bc5a9d282bc721f58c26506e28d3b4380cd9cc0ac0",
        ],
    ),
    (
        4097,
        [
            "75ffcb6f9d4dd4958de9a5d3dfa87e0a40453115dd81a5276be5ed05",
            "042a02b55ba342cdd30322331816b6e4bca073f33bdb80149dc74a6f949802f4",
            "c45d967208b168f458a7b4d09e07627e4d2aaabe7c19fe6a3c972a4514d0d1f8694731a3c58440aa13a6424136373bb1",
            "96681ccb8c621cd60d4551ccbcc61683b5cdb15db79acfaff68daa0d02b93bb29a80295574a3b082110d949456c9b73c1c3f0156b8211446ca6d333a96821138",
            "5040226b190e8c3bfe3001e045b38c57a8c4291c879b71a62196706c",
            "f85aa1a8f16ee6cad8358b6cc4118bd96850413db94fd91a078172c704a17219",
        ],
    ),
];

fn assert_output<Algorithm: Digest>(
    output: Output<Algorithm>,
    expected_bytes: &[u8],
    expected_hex: &str,
    context: &str,
) where
    Output<Algorithm>: std::fmt::LowerHex,
{
    assert_eq!(&output[..], expected_bytes, "{context}: digest bytes");
    assert_eq!(format!("{output:x}"), expected_hex, "{context}: lowercase hex");
}

fn check_answer<Algorithm: Digest>(input: &[u8], expected_hex: &str)
where
    Output<Algorithm>: std::fmt::LowerHex,
{
    assert_eq!(expected_hex.len() % 2, 0);
    let expected_bytes: Vec<u8> = (0..expected_hex.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&expected_hex[offset..offset + 2], 16).unwrap())
        .collect();
    let algorithm = std::any::type_name::<Algorithm>();
    let context = format!("{algorithm} len={} one-shot", input.len());
    assert_output::<Algorithm>(
        Algorithm::digest(input),
        &expected_bytes,
        expected_hex,
        &context,
    );
    for chunk_size in [1, 3, 7, 55, 64, 111, 128, 257] {
        let mut hasher = Algorithm::new();
        hasher.update([]);
        for chunk in input.chunks(chunk_size) {
            hasher.update(chunk);
            hasher.update([]);
        }
        hasher.update([]);
        let context = format!("{algorithm} len={} chunk={chunk_size}", input.len());
        assert_output::<Algorithm>(
            hasher.finalize(),
            &expected_bytes,
            expected_hex,
            &context,
        );
    }
}

fn check_vectors<Algorithm: Digest>(algorithm_index: usize)
where
    Output<Algorithm>: std::fmt::LowerHex,
{
    check_answer::<Algorithm>(b"abc", ABC[algorithm_index]);
    for (length, expected) in PATTERNS {
        let input: Vec<u8> = (0..*length)
            .map(|offset| ((offset * 37 + 11) % 256) as u8)
            .collect();
        check_answer::<Algorithm>(&input, expected[algorithm_index]);
    }
}

#[test]
fn sha224_independent_vectors() {
    check_vectors::<Sha224>(0);
}

#[test]
fn sha256_independent_vectors() {
    check_vectors::<Sha256>(1);
}

#[test]
fn sha384_independent_vectors() {
    check_vectors::<Sha384>(2);
}

#[test]
fn sha512_independent_vectors() {
    check_vectors::<Sha512>(3);
}

#[test]
fn sha512_224_independent_vectors() {
    check_vectors::<Sha512_224>(4);
}

#[test]
fn sha512_256_independent_vectors() {
    check_vectors::<Sha512_256>(5);
}
