import type { ComponentProps } from "react";
import Svg, { G, Mask, Path, Rect } from "react-native-svg";

import { MITSURO_CELL_PATH } from "./mitsuro-mark";

const MITSURO_WORDMARK_PATH =
  "M108 -513C63 -513 40 -488 40 -438V-61C40 -14 66 9 118 9C170 9 196 -14 196 -61V-369H721C788 -369 815 -342 815 -291V-61C815 -14 840 9 892 9C944 9 971 -14 971 -61V-278C971 -434 884 -513 721 -513ZM427 -61C427 -14 453 9 505 9C557 9 584 -14 584 -61V-253C584 -300 558 -323 506 -323C454 -323 427 -300 427 -253ZM1216 -452C1216 -499 1191 -522 1138 -522C1087 -522 1060 -499 1060 -452V-61C1060 -14 1086 9 1138 9C1190 9 1216 -14 1216 -61ZM1059 -643C1059 -596 1085 -573 1137 -573C1189 -573 1216 -596 1216 -643V-669C1216 -716 1190 -740 1138 -740C1086 -740 1059 -716 1059 -669ZM1547 -633C1547 -680 1522 -704 1470 -704C1418 -704 1391 -680 1391 -633V-513H1329C1285 -513 1264 -489 1264 -441C1264 -393 1285 -369 1329 -369H1391V-61C1391 -14 1417 9 1469 9C1521 9 1547 -14 1547 -61V-369H1609C1652 -369 1674 -393 1674 -441C1674 -489 1652 -513 1609 -513H1547ZM1864 -513C1758 -513 1701 -463 1701 -359C1701 -256 1758 -212 1864 -212H2100C2123 -212 2135 -200 2135 -176C2135 -152 2123 -139 2099 -139H1765C1721 -139 1700 -115 1700 -68C1700 -22 1721 0 1765 0H2122C2228 0 2284 -51 2284 -155C2284 -256 2232 -308 2122 -308H1886C1864 -308 1852 -320 1852 -341C1852 -363 1864 -374 1886 -374H2202C2246 -374 2267 -399 2267 -446C2267 -491 2246 -513 2202 -513ZM2497 -452C2497 -499 2472 -522 2420 -522C2368 -522 2341 -499 2341 -452V-235C2341 -79 2428 0 2591 0H2913C2958 0 2981 -26 2981 -75V-452C2981 -499 2955 -522 2903 -522C2851 -522 2825 -499 2825 -452V-145H2591C2524 -145 2497 -171 2497 -222ZM3306 -513C3143 -513 3056 -434 3056 -278V-61C3056 -14 3083 9 3135 9C3187 9 3212 -14 3212 -61V-291C3212 -342 3239 -369 3306 -369H3549C3594 -369 3617 -393 3617 -441C3617 -489 3593 -513 3546 -513ZM3927 -513C3750 -513 3655 -427 3655 -257C3655 -86 3750 0 3927 0H4051C4228 0 4323 -86 4323 -257C4323 -427 4228 -513 4051 -513ZM4051 -369C4130 -369 4166 -330 4166 -257C4166 -184 4130 -145 4051 -145H3927C3848 -145 3812 -184 3812 -257C3812 -330 3848 -369 3927 -369Z";

const ORIGINAL_O_COUNTER =
  "M4051 -369C4130 -369 4166 -330 4166 -257C4166 -184 4130 -145 4051 -145H3927C3848 -145 3812 -184 3812 -257C3812 -330 3848 -369 3927 -369Z";

type SvgStyle = ComponentProps<typeof Svg>["style"];

interface MitsuroWordmarkProps {
  width?: number;
  color?: string;
  style?: SvgStyle;
  testID?: string;
}

export function MitsuroWordmark({
  width = 220,
  color = "#c5c1c8",
  style,
  testID,
}: MitsuroWordmarkProps) {
  const height = (width * 797) / 4331;

  return (
    <Svg
      width={width}
      height={height}
      viewBox="16 -764 4331 797"
      style={style}
      testID={testID}
      accessibilityRole="image"
      accessibilityLabel="mitsuro"
    >
      <Mask
        id="mitsuro-wordmark-counter"
        x={16}
        y={-764}
        width={4331}
        height={797}
        maskUnits="userSpaceOnUse"
      >
        <Rect x={16} y={-764} width={4331} height={797} fill="#000" />
        <Path d={MITSURO_WORDMARK_PATH} fill="#fff" />
        <Path d={ORIGINAL_O_COUNTER} fill="#fff" />
        <G transform="translate(3989 -257) scale(.35) rotate(30) scale(.88) translate(-512 -512)">
          <Path d={MITSURO_CELL_PATH} fill="#000" />
        </G>
      </Mask>
      <Rect
        x={16}
        y={-764}
        width={4331}
        height={797}
        fill={color}
        mask="url(#mitsuro-wordmark-counter)"
      />
    </Svg>
  );
}
