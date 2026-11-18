import { IconArrowLeft } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

/**
 * Back to the project list. The arrow and the app's head logo make one button, so the logo in the header
 * has a job instead of standing there on its own (the OS title bar already shows the app's icon and name).
 */
export function HomeLink() {
  const { t } = useTranslation();
  return (
    <Link aria-label={t("workspace.home")} className="ide-home-button" title={t("workspace.home")} to="/">
      <IconArrowLeft size={16} />
      <img alt="" className="h-5 w-5" src="/logo-head.png" />
    </Link>
  );
}
