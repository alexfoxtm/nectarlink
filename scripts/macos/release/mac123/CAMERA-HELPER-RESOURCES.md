# Opaque camera helper resources

The normal Mac123 package carries an unmodified vendor disk image as data at
Contents/Resources/camera-helper/WaypairCameraHelper-1.3.4.dmg. Its upstream name
is LiveLoop-1.3.4.dmg. The builder does not mount/extract it, copy a vendor.app,
re-sign vendor code, launch/install/activate a helper, or change host identity.
Separate notices and source snapshots are copied as data into
Contents/Resources/camera-helper/LiveLoop-Notices/.

Official acquisition references (downloaded bytes must match these hashes):

- Vendor release: https://github.com/adrbn/liveloop/releases/tag/v1.3.4
- Opaque DMG: https://github.com/adrbn/liveloop/releases/download/v1.3.4/LiveLoop-1.3.4.dmg
- LiveLoop source tag: https://github.com/adrbn/liveloop/tree/v1.3.4
- LiveLoop source archive: https://github.com/adrbn/liveloop/archive/refs/tags/v1.3.4.tar.gz
- LiveLoop GPL license: https://github.com/adrbn/liveloop/blob/v1.3.4/LICENSE
- Sparkle source tag: https://github.com/sparkle-project/Sparkle/tree/2.10.0
- Sparkle source archive: https://github.com/sparkle-project/Sparkle/archive/refs/tags/2.10.0.tar.gz
- Sparkle full license: https://github.com/sparkle-project/Sparkle/blob/2.10.0/LICENSE

LiveLoop tag commit: 67dc46bc3dfb76cd88730bb6337966dbabc5e30b.
Sparkle tag commit: eef1a539a373c1f1a320624b1130fc5de7b2e100.
Sparkle is separately included by the vendor at2.10.0/build2064. Its full
LICENSE and official repository snapshot accompany the opaque helper.

| Resource | Bytes | SHA-256 |
| --- | ---: | --- |
| WaypairCameraHelper-1.3.4.dmg | 2329969 | 91850c858e83804128f191d9607f56cb8e2d3ac658c2ad0e6dad3270589c6946 |
| LiveLoop-Notices/GPL-3.0-LICENSE.txt | 35149 | 3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986 |
| LiveLoop-Notices/LiveLoop-1.3.4-source.tar.gz | 518788 | 525c4ab16c280784200c52d400d555707414ec02c1fed96726206ab343438a80 |
| LiveLoop-Notices/SOURCE-COMPLETENESS.json | 6050 | ac5fa9646e60e9b62a920a76902b9cbf26292b326cd33015a7f8a1561d3f20d1 |
| LiveLoop-Notices/SOURCE-PROVENANCE.json | 546 | 776c336114e57cc0b9a4303115f823270c86702259fcaf47fbad5513a7fd83d8 |
| LiveLoop-Notices/Sparkle-2.10.0-source.tar.gz | 16786836 | fd2a5de329be3619111f9a73a1adeac9725a92292f0431c93679e5e2b0d48a04 |
| LiveLoop-Notices/SPARKLE-LICENSE.txt | 6154 | 389a4e4e9a32f059775b13a06e25a591445ba229d2838d26dd3e7c0c45127cfe |
| LiveLoop-Notices/SPARKLE-SOURCE-PROVENANCE.json | 624 | 23fc732730b9acb31e46438fa609f15b0128d6a8f4f490936c79c5a91ec639b5 |
| LiveLoop-Notices/THIRD-PARTY.txt | 658 | cd1bd266000badd73a62373d661b2ca0e2e88f27e002261272d0286c2a136b0b |

The relative provenance/completeness documents and THIRD-PARTY.txt are the
reviewed payload metadata, not remote executable inputs. A complete vendor
repository snapshot is distinct from complete recursive dependency/build
material closure. Neither full recursive corresponding-source completeness
nor reproducibility of the vendor binary is claimed. Preserve those limits
when recreating the metadata, and require the exact reviewed payload hashes.

The external Root input object cameraHelperResources must bind status
ROOT_BOUND_CAMERA_RESOURCE_INPUTS, the DMG path/hash/bytes, noticesRoot, all
eight files with independent hash/byte counts, and codeSha256 for this build
recipe, mac123_guard.py and package-app.sh. Private absolute paths stay solely
in that external sealed input, never in this public recipe or copied metadata.
Missing/mismatched code/data seals, symlinks, extra notice files or signable
Mach-O inputs stop the build before output creation. The builder checks again
before copying, verifies destination bytes, and signs only the normal host.

The runtime/helper UI owns any later user-approved installation or activation.
Shipping the opaque image does not establish installation, activation,
camera-consumer acceptance, recursive source closure or binary reproducibility.
