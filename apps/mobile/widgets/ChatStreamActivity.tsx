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
  // A compact copy of apps/mobile/assets/icons/source/krusty-home-master.png.
  // Embedding the official mark makes it available inside the widget extension,
  // where the main React Native asset bundle is not guaranteed to be readable.
  const KRUSTY_LOGO =
  "data:image/png;base64,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";
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
    status === "needs_input" ? "#fbbf24" : status === "completed" ? "#34c759" : "#ff7a32";
  const timerColor = "#ff7a32";
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
      uiImage={KRUSTY_LOGO}
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
      uiImage={KRUSTY_LOGO}
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
            {chatTitle || "Krusty session"}
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
          <Image systemName="chevron.right" size={9} color="#8b93a1" />
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
        {chatTitle || "Krusty session"}
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
