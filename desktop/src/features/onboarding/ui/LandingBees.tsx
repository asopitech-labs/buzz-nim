import { NiminoMark } from "@/shared/ui/nimino-logo/NiminoMark";

type Bee = {
  top: string;
  left: string;
  size: number;
  rotate: number;
  color: string;
};

const WHITE = "#FFFFFF";
const YELLOW = "#E9E94F";

// Fixed scatter so the field doesn't shimmer between renders.
const BEES: Bee[] = [
  { top: "4%", left: "27%", size: 34, rotate: -12, color: WHITE },
  { top: "7%", left: "58%", size: 28, rotate: 18, color: YELLOW },
  { top: "5%", left: "88%", size: 32, rotate: -20, color: WHITE },
  { top: "13%", left: "12%", size: 36, rotate: 18, color: YELLOW },
  { top: "12%", left: "73%", size: 26, rotate: -8, color: WHITE },
  { top: "18%", left: "44%", size: 24, rotate: 25, color: YELLOW },
  { top: "22%", left: "90%", size: 34, rotate: 10, color: WHITE },
  { top: "28%", left: "5%", size: 28, rotate: -18, color: YELLOW },
  { top: "31%", left: "21%", size: 24, rotate: 8, color: YELLOW },
  { top: "35%", left: "84%", size: 32, rotate: -14, color: WHITE },
  { top: "45%", left: "13%", size: 32, rotate: 20, color: YELLOW },
  { top: "47%", left: "93%", size: 26, rotate: -6, color: YELLOW },
  { top: "55%", left: "30%", size: 26, rotate: -24, color: WHITE },
  { top: "57%", left: "70%", size: 34, rotate: 12, color: YELLOW },
  { top: "63%", left: "8%", size: 34, rotate: 16, color: WHITE },
  { top: "66%", left: "88%", size: 28, rotate: -10, color: YELLOW },
  { top: "72%", left: "48%", size: 26, rotate: 22, color: YELLOW },
  { top: "76%", left: "18%", size: 32, rotate: -16, color: WHITE },
  { top: "80%", left: "64%", size: 28, rotate: 8, color: YELLOW },
  { top: "86%", left: "34%", size: 34, rotate: -20, color: WHITE },
  { top: "88%", left: "80%", size: 32, rotate: 14, color: YELLOW },
  { top: "92%", left: "10%", size: 26, rotate: -8, color: YELLOW },
  { top: "3%", left: "42%", size: 22, rotate: 14, color: WHITE },
  { top: "9%", left: "5%", size: 24, rotate: -22, color: YELLOW },
  { top: "16%", left: "62%", size: 30, rotate: -4, color: YELLOW },
  { top: "20%", left: "30%", size: 22, rotate: 12, color: WHITE },
  { top: "26%", left: "52%", size: 26, rotate: -14, color: YELLOW },
  { top: "33%", left: "68%", size: 22, rotate: 24, color: WHITE },
  { top: "40%", left: "40%", size: 24, rotate: -10, color: YELLOW },
  { top: "42%", left: "78%", size: 28, rotate: 6, color: YELLOW },
  { top: "52%", left: "55%", size: 22, rotate: -18, color: WHITE },
  { top: "60%", left: "42%", size: 28, rotate: 10, color: YELLOW },
  { top: "68%", left: "26%", size: 24, rotate: -6, color: WHITE },
  { top: "70%", left: "76%", size: 30, rotate: 18, color: YELLOW },
  { top: "82%", left: "6%", size: 28, rotate: 22, color: WHITE },
  { top: "84%", left: "50%", size: 24, rotate: -12, color: YELLOW },
  { top: "94%", left: "60%", size: 28, rotate: 16, color: YELLOW },
  { top: "95%", left: "90%", size: 22, rotate: -24, color: WHITE },
];

export function LandingBees() {
  return (
    <div
      aria-hidden
      className="pointer-events-none absolute inset-0 overflow-hidden"
    >
      <span className="absolute left-6 top-12 block w-11 text-[#231E1E]">
        <NiminoMark className="h-auto w-full" />
      </span>
      {BEES.map((bee) => (
        <span
          key={`${bee.top}-${bee.left}`}
          className="absolute block"
          style={{
            top: bee.top,
            left: bee.left,
            width: bee.size,
            color: bee.color,
            transform: `rotate(${bee.rotate}deg)`,
            opacity: 0.9,
          }}
        >
          <NiminoMark className="w-full" />
        </span>
      ))}
    </div>
  );
}
