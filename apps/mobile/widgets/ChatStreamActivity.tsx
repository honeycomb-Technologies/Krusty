import { createLiveActivity } from "expo-widgets";
import { Button, HStack, Image, Spacer, Text, VStack } from "@expo/ui/swift-ui";
import {
  aspectRatio,
  clipped,
  cornerRadius,
  font,
  foregroundStyle,
  frame,
  layoutPriority,
  lineLimit,
  monospacedDigit,
  opacity,
  padding,
  resizable,
} from "@expo/ui/swift-ui/modifiers";
import type { LiveActivityComponent } from "expo-widgets";

export interface ChatStreamProps {
  chatTitle: string;
  status: "working" | "needs_input" | "completed";
  startedAtMs: number;
  elapsedSeconds: number;
  toolCount: number;
  filesAdded: number;
  filesRemoved: number;
  toolApprovalId?: string;
  toolApprovalName?: string;
  toolApprovalSessionId?: string;
}

const ChatStreamActivity: LiveActivityComponent<ChatStreamProps> = (props) => {
  "widget";

  // Expo serializes only this function body into the widget extension. Keep
  // every runtime value used by the layout inside this boundary.
  // A compact copy of apps/mobile/assets/icons/source/mitsuro-app-icon-master.png.
  // Embedding the official Mitsuro mark makes it available inside the widget extension,
  // where the main React Native asset bundle is not guaranteed to be readable.
  const MITSURO_LOGO =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAADAAAAAwCAYAAABXAvmHAAAABGdBTUEAALGPC/xhBQAAACBjSFJNAAB6JgAAgIQAAPoAAACA6AAAdTAAAOpgAAA6mAAAF3CculE8AAAARGVYSWZNTQAqAAAACAABh2kABAAAAAEAAAAaAAAAAAADoAEAAwAAAAEAAQAAoAIABAAAAAEAAAAwoAMABAAAAAEAAAAwAAAAANs3bAwAAAHNaVRYdFhNTDpjb20uYWRvYmUueG1wAAAAAAA8eDp4bXBtZXRhIHhtbG5zOng9ImFkb2JlOm5zOm1ldGEvIiB4OnhtcHRrPSJYTVAgQ29yZSA2LjAuMCI+CiAgIDxyZGY6UkRGIHhtbG5zOnJkZj0iaHR0cDovL3d3dy53My5vcmcvMTk5OS8wMi8yMi1yZGYtc3ludGF4LW5zIyI+CiAgICAgIDxyZGY6RGVzY3JpcHRpb24gcmRmOmFib3V0PSIiCiAgICAgICAgICAgIHhtbG5zOmV4aWY9Imh0dHA6Ly9ucy5hZG9iZS5jb20vZXhpZi8xLjAvIj4KICAgICAgICAgPGV4aWY6Q29sb3JTcGFjZT4xPC9leGlmOkNvbG9yU3BhY2U+CiAgICAgICAgIDxleGlmOlBpeGVsWERpbWVuc2lvbj4xMDI0PC9leGlmOlBpeGVsWERpbWVuc2lvbj4KICAgICAgICAgPGV4aWY6UGl4ZWxZRGltZW5zaW9uPjEwMjQ8L2V4aWY6UGl4ZWxZRGltZW5zaW9uPgogICAgICA8L3JkZjpEZXNjcmlwdGlvbj4KICAgPC9yZGY6UkRGPgo8L3g6eG1wbWV0YT4Kwe07qQAACnlJREFUaAWdmVmTHLcRhDHHHlpySYUYoRc/Ovzi//+bfIRoyRK5x+xczi8T1Y3pXVohYTkNoM4sVAF9cPW3v/793FatrVZrdfpb1Q+aKQxaGvNlOzf/nUU/a6T+3E6hnfr8rDk80U+vaChE7lRynkdeSu1U84EPCpHblkFgjsAroJkL8kvwmWHE+isc0nQ9r9WfugJ0yZqJD4ZMurUYQNF2zIuhLmPtqFtnYop/bttplRGXzWSgspG5TdtfAEjUgAwDQBp4bEaY5/Ms21UlhAMBGjEIVKa5shj8uwgkal1OE7fIKwNwcYbq2EPucyJDzg0pNV8wwkA9/zQ0uaj20VfQDF16YGH5KunXLV4UBnoVOEOpAIeetoXvny4VBIQJfKMcIoMmsnPLjCv2VN5xqLE5EB08q85Km2Bp65jHNEEiG2DI0cryPEalwMPf2oGohmaDGb8dDDYNzYgNqYOGao59B0BWrzt36SAhnjpA9Fk0qSto8FDxhcFCbmSI5+XFENamfhgQzroHOJ1Uoq3W+kG3HpeMMdUtdX4yOCEe+B7O0jXFepmYuBNzMeh7AHmjvrgiC0hzwzbgybp5Om20KkkrVlQO63W7urlpV1c+5NrheGzPj0/tebfDZBr2lKIqrCJf9JYZxC+YYOvH6ExHQ0bVYZo//qVBNyWaInK2W7jnHL3N9VW7ut629UYnGQQ1HXXt7v1d24r3+PDYDvsX0//fBU/ZM0spbM715U28FBnniINjCX6UweBaq76+0o/S0vhwOLSvX7+2o1b/7u6u3Sgj2+22vb9/13bP2/b09NQO7Xhp5k/MkuNJsUfWj62QWQkFoNU+KxJ62rwG4m0UoUo9C75qv/zyc/vPz5/bfn+wLEHdv79vP/zwqW02W2finTL0/PzcdruXZNKSf/yyNbiuV+MRnKtE/NPiBkSMpPi84o6r42y9MaCfPv/knk2+2WwmRF+UjYfHx/bx48f24f6Ds3StrKwls3vevSqrt8sHcyM67wGJCs25H3MOgiON4lcHnTad4wLLM82R5xL9Afx0OrV//+dz+/Lli2Upp6p/E9BPepSdX1xa33/8vt3c3rr0KK+NMkI2Tqej7VoP13FfXZkLW7y5hBDswNmc3kKKwZrmhX8QWB66XPMC+utvv7b/CtRR9EvgKjf0Fw0Z9geZulUAHz58zIY/rdv1zbV5+5exrHoECzuZagH7+aeVhtQRkxH9GYD04R3PR4HMpmMzvuhI/Fm1vlNfwdQqp38dgMux+1lpsdgD/N69e6ffe2fJi6eM7EQ/a6FocwjzyAxdtAcgyqo6YLsJOQWy7ixKZH/a+4YG2M9avYeHr3ZYq/42eKOtZZF8NyjblC26AOa0emR/fPjQtldX7eXl5H63q1MKPf34138BqrMDFlQM+YdxHoU159md3/744uMQ2X/+6x+udd+VeYcY6mRcecb102AeK5zQC4JA9EDI6H6/n/iSjBCOAe5Zn3T1NWkCMuxEp56Xjol2Ul0e7eTrl9/sgA03tgJODomn5jWeAYcf3QqkgxQROTLBsUvWaTOSzEwcLtoDETKNFJ9AQC9Kt+1Nq5eUF91BAw5ngxVEUTONSwENSCRZHEqI0qFl7OFwCXAqgbcwkM1tHMcevOyBiRfUnEGBEQar4Q3laQDMhr81msEjQeDGdAHqbV37w2dWN2jlG/3YQC/YtiaAqTgak4Cwcxq5zFxWlw6XWYC7pNW8zF9aeHvmDFBCHQdYgmceoYmcAgjLnjVmhtM8MmTVWBHKaAoSbTVUC2AoIy12q2Tip/vquqWz7O2PAKaG3qg7j3Ujoy4jmdKZmVAJKSXEaOb9HvgCPjsuP9jIr/zGe7z5Svn4HsACRi/97L90nAGvonlATI0zdQFhwMdpUpqAa5OWGVElV9kA/NubtIIoPeYJpij0ZJtFW+uYrmCLb3GofTBngLJBSvbM8yQrgEGvwJCBMogC8nU6VSDJwBhUN44LKXT/NpPxTKsSWum4hpcfgdKqj52tFtcrbVYFIaF6eKvVP1OTk24GcdyzoQnUnDbhk5HIYL3GBZQeudFW5inZLFr4JXPZY9PPQimbMCH2KuqnUfaAsyBn+CxgKZkOSBOSFlDfKqHij32Nux3Zp/55AthsQotNI3t1UQZ6mo3fELqQlMlNr8ekHaEYDXicJQOjkwrwlbey3FeervRMwjZ/ynYdHJq6zXIh1LxnACLge+Nu2af+LtkNljGkymGBXfZlirwAamzl/LKXFGL6ZQ/ovSBOuq8x2LLHfUAa842ruxF4dInBe0CP0hjzXzeaDJAhSgrnVTbWsiEH1U2CrKtitY/HHlCam1cAS3k5Dx17uZHJJ26x6v0QVBD8TMKK8HJeG7T6mHn7mozMwWCr/+sKC/Cw5ZfX0OPxoL5eVQJ+zkbmNiZLehrFUIA6erzYU04BF4BIvLhc6VmddmlsBFK82CzbUy+707j7zTxnPy9KWwNPIAUSmdEv9E6qR4lxpSxbKfGqsyrPz09to+d2PpHwFkVW6jm+6h/NefyGTS8OQRqOHfmpUwReL2+ub9oTfhREbeICX8FYM/F4MeZnIZuLU/hsvrRzu7u9aye+rr3sfJe8++7OJXXxpQ0dqdTKdOVF1z2LWpkgqywKJ8/z7tkL9hr8bHcZ0OrHH//C5x41XfzPkwSQocdKml71dm1/4IW7acX4dHjt0qK82Bf85lZ2LtbObECQvfd6D95sN7LL14gcnbzwV3Blq0BXj8Us1Pg0inSWPk40KQiAp10rxdvtlb517v31AKe3N7f+PTw+mDYHEp1crW5gjFjx75TFw2HvNzyAAZyDgjYDnceFIXyLGZVKSJu1r7FTYHKgJ55cR8PbzZX+F2kjSWVF77Bsvo/6PMIb28PDg4GwwtUAxI86v7+/F3nVgfO6epiCGH0YcI/+Ypyll6iY+rd4IzNN5gfQdtct9ZyUlMNU2QAO8NTzp0+fHATvtmSIBp2vcXzA2mvV+XjFau9U8yWDXK189aZ1oCMf/4nD34XE4kaEhK55iMu4fyyqrgdmQctaR5YwxkkFGMasMt95+EBFJviEyCbdu0y0WXXS8PWh2gXgIDNrphdgyDUO5jwLgYRNCJvbcm/JhImmTJxUWKRExOf+RA2ftE/0dU2ry7P87Xe3liEwVj0fc/lgFUvVl6EB+yQzA0aqwDPylPsAaTZ01t+PFQRz2SQ+kTQobYSCxR130KfHgz9KcUrlYZfT66U98zld/Lfu4t8OZAY8grfTjsH3AZLR119OE0ywhcq4gHowiwysORKOW34F9q06t8lxyXFxMX8LPEgRzAXxxY0MXvZBVrwrzMuf8KQ4x2BrWHQbQYzAi08/yngOosnMAngE0IqI5Urm4j4AB1g9iAH0YH3yM/mzRi5LYAMr7hdKhnRBK2BoLsYhhc64I/E7McCTvdkaYC7vrNb6Q5dvBfR7wA1vglKBhDDi7CVk8b5vk4FC+S0Axf9jfQEZtZa0ca4xovPlYpErkG9mIAfRZUCj698fjwBG6XEliz4ChzbOR/kap0dSmzgKXOfT83VJwf9zrZwutZcBLuVG/sjLuKz9DyK0tltrRK2PAAAAAElFTkSuQmCC";
  const CODE_ICON =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAEgAAABICAYAAABV7bNHAAAAAXNSR0IArs4c6QAAAGxlWElmTU0AKgAAAAgABAEaAAUAAAABAAAAPgEbAAUAAAABAAAARgEoAAMAAAABAAIAAIdpAAQAAAABAAAATgAAAAAAAADYAAAAAQAAANgAAAABAAKgAgAEAAAAAQAAAEigAwAEAAAAAQAAAEgAAAAARDxGgQAAAAlwSFlzAAAhOAAAITgBRZYxYAAABNtJREFUeAHtm71PFEEUwGfuvAO1UIxacZUJldhopYUxkqAIEdEQDUbkDxA7ba/VTjor1EhICCpGRDFaWGinhVoYEiuwUSKa+AF33o7zzgzZ3Xu7N7vzwRYzCdndtzPv43ezM/teFkJccwQcAUfAEXAEHAFHwBFwBBACFJFlTjQ5/ewAzeduMkI6ucPvGa1dOtff/dqGo5kHdPfeXHs+X/hICd0qgHBQK5XKn87hwb7PQmbqmDOlWJfefK5w0Q8H9PJfta2l0HJGl404PZkHxOH0xwVg+t4m0wZU9E/OzJf4+P2YjgpjLzC5blmmZxBluZNYwIyRTxcGuj9g93TLMg2IL8bo48XXoBndIKL0ZRbQxMRsG6X0MOa4x5gDRLcUT3A4DWskY+zLwrtXVt6B4MfJ7AzKRe5e9FG5XPawmWVClklA4+PjrYySY1jAlNlbf8B+wxTGnLIt27yt1MVtrr85C/uMsF+rP5aei2sbRyVAY2NzLTtLxWt8Gg6Ds3ze316ofrtSHhysqDjPZ08/lgPx2fN0ZGRkNUp3eWqq2FHYcd3vz/Ji5eroaM9a1Jhm8tSPGMDZVSo+yFFymb/7b4c/OAcHmxmNu8/XF+4T68P6eCR+96rDCfkDPoKvmD4ZWSpAAg6l5HjYiPj1wnLZ6459hw7y7X030v8v+115jMjXRZht8FEFUmJAcXDWPVU4yVE89+Lb+8uhod6VNKpVICUCJAMH1qE0QYgxKm/PcbbTQpIGJAOH50hPYFEUwSY93rk/v5cHsgcbx6j3EJP7ZWAbfPDL/OdpIEkBkoXzdbFySmXHKNAcmnsRRt7wCuKiP1jsHGyDDzohNQVkCw4EHFX74e8/0rmXbkixgGzCqdd+KF77qTJPGhCA1gkpEpBNOPXZo7n2owsSCsg2HACksnvBeKzpgNQAaCPgmKz9qEIKANoIOPDLm679qEAKAILEE94VsOkKMtg+VbdyTLeN2o8sJGDg9zEAiF/Us3J/B3FuCo7N2o8MpDADpXKHgKdyzFLtB4sjMINM5DKYUb8Maj/+a3HerPYj+iU5yqyxYQYBQCZymbgAVGo/cXqxezJwYBkJ55IBQDLPaJqED3MYZCq1nyidmFwWDrYBBQCBcpuQTNR+woBU4ICuBkAgtAXJxNsz+C+aKhzQgwKCG6YhqdZ+wMe4pgMO6I8EBDdNQlKt/YB/UU0XHNAfCwg6mIKko/YD/oWbTjiguykg6KQbks7aD/gnmm44oFcKEHSUhRTOZWBsuJn67sdELikNCIKUgcQVRuZzApSp3SvOdtpcMhEgWUgCBHY0WfvB7IEsLRwYmxgQDIqbSeFcBvr7m8naD2ZbBQ74nQoQDBSQPEZu8Hrpd/iDc/h4Ae5HNZO1H7Ad9gdLH6J8w+TYRxRYPy0yqP20trUvh797riv3SN/Zga5ZLYY0KrFaD8p67QfjmvoRw5Q1k9ms/TTzRfa+NUA2az+ywcv0swbIVu1HJugkfawBslH7SRK4bF9rgEy9PcsGmrafFUCmaz9pg5cZZwVQkdKjqDOS3/2gYy0JrQCKiiXJdz9ROkzLrQBaq65N8zUo+AEmIz9rXvWW6QBV9VsB9P9/S2u9PF97y5PHav1Y846cP92zpBqAG+8IOAKOgCPgCDgCjoAj4AhsCIF/S/I8n8HpBxIAAAAASUVORK5CYII=";
  const TOOLBOX_ICON =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAEgAAABICAYAAABV7bNHAAAAAXNSR0IArs4c6QAAAGxlWElmTU0AKgAAAAgABAEaAAUAAAABAAAAPgEbAAUAAAABAAAARgEoAAMAAAABAAIAAIdpAAQAAAABAAAATgAAAAAAAADYAAAAAQAAANgAAAABAAKgAgAEAAAAAQAAAEigAwAEAAAAAQAAAEgAAAAARDxGgQAAAAlwSFlzAAAhOAAAITgBRZYxYAAABOhJREFUeAHtnM1PFDEUwNtlWaPxIMab8abyH+hJJRERjQbWmBVC8KRnb3DlKl7krCcNAYlxMRpFxAT1pH+CejPejF6MBli2vjfjrG2n3c5Hux/QSTYz89q+tr990/a10yHEH56AJ+CQAHWoW1C9UF09Syi9Rgk9TRg5TCjZL0Qw3TDyC9J8Y4S9I4w9Gi8PvTElsRHuHND849VjhR56j1J6xkaBIx2Msbf1bXZz4urQ50jm4uwU0MLyyilCik8hkz4XhWeE/CSkNjI+OvzehX7U6QxQYDnFwgdXcCIgCKleq590ZUmFKCPb5+CxcmQ5fFnxD8C8eJnNayeAsEG23eY0qzTmFXQCzSJlDCtmTNc8GfRWygiMbRLCprZ/bz6YmLgE7UfyY37+eV/PvtJ1aBVmoTcsxVKGeVrv2ZwACrryWA1QwKbGykNzyiCD8B/QucXqKsSkd+Xo+jzlmOnunTxiwThHUQ60HIU4lUirA8dWDg43gDSDwLSPlaq+Wh2aPFU60sjcAEpTgg6P6wEZ/iDrA8Wwiy+sGfJ1EsxYfdC2j2YN0MOllaPF3p77rRz/qCijj1bb2r4xWRn+ogpPK7MCaKH6egDGJk9wVJu2AC7iBz4aY1fGy+fW8+rPDSh4pEjhGXh1e/MWxmp6Rv4wUr+c95HLBahj4USkLUDKDKjj4ViClAmQGQ7bgFm/6Sw+V1SvpOeGj0bpbXBB9ijT5bCk1ICSwamXx8rnXyoL60i4WH11gdBC1TakVIA6FU7E3AWkxIA6HY4rSIkAdQscF5CMgLoNjm1ITQF1KxybkLSAuh2OLUhKQOhbwRTmC737gOOc1nflUaXTnpP1buyiyneLAQq88lLxIwRoHM/ughPBNEFCB7e2WTshzwLEJsyCKYsdBgchBQNXsHpYONiIoPFnNAisOy/DawFQ0O5o19C703L4ChshKdbXxGUf3XoWUk/R5swsLZWO9x6cpZTAOhb8Z4w8+LT1Y2qmUoF1sfxHHv0ICR63st4toRUoYWN9TbAg7doSOJ5pfKv+0oE7BUpuodniD68RWH40oYa8+kNLYtOq8sgzogIge+tZhUk588iaZHm2+/z6k66viYA0a0vatShN7VQ9oEqmSW4Uq3SpZM0UaeskMRABNdO4S8M8IMMfD5b5/1hcXoPxkj/GRgcbXLwFGezBA/KADAQMwd6CPCADAUOw6ItpIvOtuiaKINb1hmn1CEq5G1v6dXq4rERvng/w1yEB3wYZLMEJoOD1EyljlUyKkvhWpUslS6ywSUQ3gGD+R84T54RkWdZ7lS6VLKt+Pl2iRppPkOT6+9fN6UNHSvBOlThhliRtkjiu9fNlaPgcKNS16rZ6Hz7jTrhOUl8nj1gnVN5WGTwgA0kPyAMyEDAEewtKBQh3FisOfA9QIe5qkbZOEgPRgmDbtarW4UY2VUj3yrR1khgIA0Xckw6Lh/3xatNZ3MjWirdW43nblTTeisWdi4oj2JfPyYWBYrg2356NKFyZ2nopb4gRAGHJ4N2gdXn5ta0lbmHmuBEG3hEa4LMU2yAIwa8ZuPKM+Yw77RrrjHWXyxUDFG7Qr43sJkhhXWsjqo8TxAAhQfzUA37NAE1OJrrT7oNvgEBddZ+3iLVBMgBsuGHeIvtXW2SF7b5v01dk2l1tn78n4Al4Ap7AbiTwF7tHQuje7xuFAAAAAElFTkSuQmCC";

  function formatElapsed(seconds: number): string {
    const minutes = Math.floor(Math.max(0, seconds) / 60);
    const remainder = Math.max(0, seconds) % 60;
    return `${minutes}:${String(remainder).padStart(2, "0")}`;
  }

  const {
    chatTitle,
    status,
    startedAtMs,
    elapsedSeconds,
    toolCount,
    filesAdded,
    filesRemoved,
    toolApprovalId,
    toolApprovalName,
    toolApprovalSessionId,
  } = props;
  const statusColor =
    status === "needs_input" ? "#b89a61" : status === "completed" ? "#7f9a86" : "#75617e";
  const timerColor = "#b89a61";
  const statusLabel =
    status === "needs_input" ? "Needs input" : status === "completed" ? "Completed" : "Working";
  const compactStatusSymbol =
    status === "needs_input"
      ? "exclamationmark.circle.fill"
      : status === "completed"
        ? "checkmark.circle.fill"
        : "circle.fill";
  const elapsed = formatElapsed(elapsedSeconds);
  const timerInterval = {
    lower: new Date(startedAtMs),
    upper: new Date(startedAtMs + 24 * 60 * 60 * 1000),
  };
  const bannerTimer =
    status === "completed" ? (
      <Text modifiers={[font({ size: 15, weight: "semibold" }), monospacedDigit(), foregroundStyle(timerColor)]}>
        {elapsed}
      </Text>
    ) : (
      <Text
        timerInterval={timerInterval}
        countsDown={false}
        modifiers={[font({ size: 15, weight: "semibold" }), monospacedDigit(), foregroundStyle(timerColor)]}
      />
    );
  const compactTimer =
    status === "completed" ? (
      <Text modifiers={[font({ size: 11, weight: "semibold" }), monospacedDigit(), foregroundStyle(timerColor)]}>
        {elapsed}
      </Text>
    ) : (
      <Text
        timerInterval={timerInterval}
        countsDown={false}
        modifiers={[font({ size: 11, weight: "semibold" }), monospacedDigit(), foregroundStyle(timerColor)]}
      />
    );
  const approvalTarget =
    toolApprovalId && toolApprovalSessionId
      ? `${encodeURIComponent(toolApprovalSessionId)}:${encodeURIComponent(toolApprovalId)}`
      : null;

  const logo = (
    <Image
      uiImage={MITSURO_LOGO}
      modifiers={[
        resizable(),
        aspectRatio({ ratio: 1, contentMode: "fit" }),
        frame({ width: 28, height: 28 }),
        cornerRadius(8),
        clipped(),
      ]}
    />
  );
  const compactLogo = (
    <Image
      uiImage={MITSURO_LOGO}
      modifiers={[
        resizable(),
        aspectRatio({ ratio: 1, contentMode: "fit" }),
        frame({ width: 17, height: 17 }),
        cornerRadius(5),
        clipped(),
      ]}
    />
  );
  const toolIcon = (
    <Image
      uiImage={TOOLBOX_ICON}
      modifiers={[
        resizable(),
        aspectRatio({ ratio: 1, contentMode: "fit" }),
        frame({ width: 12, height: 12 }),
      ]}
    />
  );
  const codeIcon = (
    <Image
      uiImage={CODE_ICON}
      modifiers={[
        resizable(),
        aspectRatio({ ratio: 1, contentMode: "fit" }),
        frame({ width: 12, height: 12 }),
      ]}
    />
  );
  const metrics = (
    <HStack spacing={16}>
      <HStack spacing={6}>
        {toolIcon}
        <Text modifiers={[font({ size: 10, weight: "medium" }), opacity(0.76)]}>{toolCount} tools</Text>
      </HStack>
      <HStack spacing={6}>
        {codeIcon}
        <HStack spacing={3}>
          <Text modifiers={[font({ size: 10, weight: "medium" }), monospacedDigit(), foregroundStyle("#34c759")]}>+{filesAdded}</Text>
          <Text modifiers={[font({ size: 10, weight: "medium" }), monospacedDigit(), foregroundStyle("#ff5b62")]}>−{filesRemoved}</Text>
        </HStack>
      </HStack>
    </HStack>
  );

  return {
    banner: (
      <VStack alignment="leading" spacing={10} modifiers={[padding({ horizontal: 14, vertical: 13 })]}>
        <HStack spacing={10}>
          {logo}
          <Text modifiers={[font({ size: 13, weight: "semibold" }), lineLimit(1), layoutPriority(1)]}>
            {chatTitle || "Mitsuro session"}
          </Text>
          <Spacer />
          <VStack alignment="trailing" spacing={3} modifiers={[frame({ minWidth: 58, alignment: "trailing" })]}>
            {bannerTimer}
            <HStack spacing={4}>
              <Image systemName="circle.fill" size={5} color={statusColor} />
              <Text modifiers={[font({ size: 9, weight: "medium" }), foregroundStyle(statusColor)]}>{statusLabel}</Text>
            </HStack>
          </VStack>
        </HStack>
        <HStack>
          {metrics}
          <Spacer />
          <Image systemName="chevron.right" size={9} color="#9e98a3" />
        </HStack>
        {status === "needs_input" && approvalTarget && (
          <VStack spacing={6}>
            <Text modifiers={[font({ size: 11 }), opacity(0.7), lineLimit(1)]}>
              {toolApprovalName || "Tool"} needs permission
            </Text>
            <HStack spacing={10}>
              <Button target={`deny:${approvalTarget}`} role="destructive" label="Deny" modifiers={[frame({ maxWidth: 999 })]} />
              <Button target={`approve:${approvalTarget}`} label="Approve" modifiers={[frame({ maxWidth: 999 })]} />
            </HStack>
          </VStack>
        )}
      </VStack>
    ),
    compactLeading: (
      <HStack spacing={6}>
        {compactLogo}
        <Image
          uiImage={CODE_ICON}
          modifiers={[
            resizable(),
            aspectRatio({ ratio: 1, contentMode: "fit" }),
            frame({ width: 10, height: 10 }),
          ]}
        />
        <HStack spacing={3}>
          <Text modifiers={[font({ size: 9, weight: "medium" }), monospacedDigit(), foregroundStyle("#34c759")]}>+{filesAdded}</Text>
          <Text modifiers={[font({ size: 9, weight: "medium" }), monospacedDigit(), foregroundStyle("#ff5b62")]}>−{filesRemoved}</Text>
        </HStack>
      </HStack>
    ),
    compactTrailing: (
      <HStack spacing={4}>
        <Image systemName={compactStatusSymbol} size={7} color={statusColor} />
        {compactTimer}
      </HStack>
    ),
    minimal: compactLogo,
    expandedLeading: logo,
    expandedCenter: (
      <Text modifiers={[font({ size: 12, weight: "semibold" }), lineLimit(1)]}>
        {chatTitle || "Mitsuro session"}
      </Text>
    ),
    expandedTrailing: (
      <VStack alignment="trailing" spacing={3}>
        {compactTimer}
        <HStack spacing={3}>
          <Image systemName="circle.fill" size={4} color={statusColor} />
          <Text modifiers={[font({ size: 8, weight: "medium" }), foregroundStyle(statusColor)]}>{statusLabel}</Text>
        </HStack>
      </VStack>
    ),
    expandedBottom: (
      <VStack alignment="leading" spacing={8}>
        {metrics}
        {status === "needs_input" && approvalTarget && (
          <HStack spacing={8}>
            <Button target={`deny:${approvalTarget}`} role="destructive" label="Deny" modifiers={[frame({ maxWidth: 999 })]} />
            <Button target={`approve:${approvalTarget}`} label="Approve" modifiers={[frame({ maxWidth: 999 })]} />
          </HStack>
        )}
      </VStack>
    ),
  };
};

export default createLiveActivity("ChatStreamActivity", ChatStreamActivity);
